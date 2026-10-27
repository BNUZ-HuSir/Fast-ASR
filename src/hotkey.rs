//! Delayed, standalone Ctrl gesture. The hook never performs audio or inference work.
use std::{cell::RefCell, mem::size_of, ptr::null, ptr::null_mut, time::Instant};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub const CTRL_RELEASE: u32 = WM_APP + 20;
pub const CTRL_CANCEL: u32 = WM_APP + 21;
pub const CTRL_FAILED: u32 = WM_APP + 22;
const OWN_INPUT: usize = 0x46415352;
pub const TEST_INPUT: usize = 0x46415354;
const HOLD_MS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    vk: u16,
    scan: u16,
    extended: bool,
    up: bool,
}
impl Key {
    fn ctrl(self) -> bool {
        [VK_CONTROL, VK_LCONTROL, VK_RCONTROL].contains(&self.vk)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    Pending,
    Recording,
    Cancelled,
    Forwarding,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Effect {
    Pass,
    Suppress,
    Tap(Key),
    Combo(Key, Key, bool),
    Release,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Poll {
    None,
    Start,
    Forward(Key),
}
struct Gesture {
    phase: Phase,
    ctrl: Key,
    since: u64,
}
impl Default for Gesture {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ctrl: Key {
                vk: VK_LCONTROL,
                scan: 0,
                extended: false,
                up: false,
            },
            since: 0,
        }
    }
}
impl Gesture {
    fn input(&mut self, key: Key, now: u64, allowed: bool, other_down: bool) -> Effect {
        if self.phase == Phase::Idle {
            if key.ctrl() && !key.up && allowed && !other_down {
                self.phase = Phase::Pending;
                self.ctrl = key;
                self.since = now;
                return Effect::Suppress;
            }
            return Effect::Pass;
        }
        if self.phase == Phase::Forwarding {
            if key.vk == self.ctrl.vk && key.up {
                self.phase = Phase::Idle;
            }
            return Effect::Pass;
        }
        if key.vk == self.ctrl.vk {
            if !key.up {
                return Effect::Suppress;
            }
            let previous = self.phase;
            self.phase = Phase::Idle;
            return match previous {
                Phase::Pending => Effect::Tap(self.ctrl),
                Phase::Recording => Effect::Release,
                _ => Effect::Suppress,
            };
        }
        if !key.up {
            if self.phase == Phase::Recording && key.vk == VK_ESCAPE {
                self.phase = Phase::Cancelled;
                return Effect::Cancel;
            }
            if self.phase == Phase::Cancelled && key.vk == VK_ESCAPE {
                return Effect::Suppress;
            }
            let cancel = self.phase == Phase::Recording;
            self.phase = Phase::Forwarding;
            return Effect::Combo(self.ctrl, key, cancel);
        }
        Effect::Pass
    }
    fn poll(&mut self, now: u64, allowed: bool) -> Poll {
        if self.phase != Phase::Pending {
            return Poll::None;
        }
        if !allowed {
            self.phase = Phase::Forwarding;
            return Poll::Forward(self.ctrl);
        }
        if now.saturating_sub(self.since) >= HOLD_MS {
            self.phase = Phase::Recording;
            return Poll::Start;
        }
        Poll::None
    }
}
struct Context {
    hwnd: isize,
    clock: Instant,
    gesture: Gesture,
    allowed: bool,
    test_input: bool,
    trace: Vec<String>,
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }

fn replay(keys: &[Key]) -> bool {
    let inputs: Vec<_> = keys
        .iter()
        .map(|key| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key.vk,
                    wScan: key.scan,
                    dwFlags: if key.up { KEYEVENTF_KEYUP } else { 0 }
                        | if key.extended {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            0
                        },
                    time: 0,
                    dwExtraInfo: OWN_INPUT,
                },
            },
        })
        .collect();
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        ) == inputs.len() as u32
    }
}
unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(null_mut(), code, w, l);
        }
        let event = &*(l as *const KBDLLHOOKSTRUCT);
        let (effect, hwnd) = CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return (Effect::Pass, 0);
            };
            if event.flags & LLKHF_INJECTED != 0
                && !(context.test_input && event.dwExtraInfo == TEST_INPUT)
            {
                return (Effect::Pass, context.hwnd);
            }
            let key = Key {
                vk: event.vkCode as u16,
                scan: event.scanCode as u16,
                extended: event.flags & LLKHF_EXTENDED != 0,
                up: event.flags & LLKHF_UP != 0,
            };
            let other_down = key.ctrl()
                && (8..=254u16).any(|vk| {
                    ![VK_CONTROL, VK_LCONTROL, VK_RCONTROL].contains(&vk)
                        && GetAsyncKeyState(vk as i32) < 0
                });
            let effect = context.gesture.input(
                key,
                context.clock.elapsed().as_millis() as u64,
                context.allowed,
                other_down,
            );
            if context.test_input {
                context.trace.push(format!(
                    "key={key:?} flags={} other={other_down} effect={effect:?}",
                    event.flags
                ));
            }
            (effect, context.hwnd)
        });
        // Drop the RefCell borrow before SendInput or window messages can reenter.
        match effect {
            Effect::Pass => CallNextHookEx(null_mut(), code, w, l),
            Effect::Suppress => 1,
            Effect::Tap(ctrl) => {
                if replay(&[ctrl, Key { up: true, ..ctrl }]) {
                    1
                } else {
                    PostMessageW(hwnd as HWND, CTRL_FAILED, 0, 0);
                    CallNextHookEx(null_mut(), code, w, l)
                }
            }
            Effect::Combo(ctrl, key, cancel) => {
                if cancel {
                    PostMessageW(hwnd as HWND, CTRL_CANCEL, 0, 0);
                }
                if replay(&[ctrl, key]) {
                    1
                } else {
                    PostMessageW(hwnd as HWND, CTRL_FAILED, 0, 0);
                    CallNextHookEx(null_mut(), code, w, l)
                }
            }
            Effect::Release => {
                PostMessageW(hwnd as HWND, CTRL_RELEASE, 0, 0);
                1
            }
            Effect::Cancel => {
                PostMessageW(hwnd as HWND, CTRL_CANCEL, 0, 0);
                1
            }
        }
    }
}

pub struct CtrlHotkey {
    handle: HHOOK,
}
impl CtrlHotkey {
    pub fn install(hwnd: HWND, test_input: bool) -> Result<Self, String> {
        let occupied = CONTEXT.with(|cell| cell.borrow().is_some());
        if occupied {
            return Err("Ctrl 快捷键已注册".into());
        }
        CONTEXT.with(|cell| {
            *cell.borrow_mut() = Some(Context {
                hwnd: hwnd as isize,
                clock: Instant::now(),
                gesture: Gesture::default(),
                allowed: true,
                test_input,
                trace: Vec::new(),
            })
        });
        let handle = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(null()), 0)
        };
        if handle.is_null() {
            CONTEXT.with(|cell| *cell.borrow_mut() = None);
            return Err(format!(
                "无法启用 Ctrl 长按快捷键（Windows {}），请使用麦克风按钮",
                unsafe { GetLastError() }
            ));
        }
        Ok(Self { handle })
    }
    pub fn diagnostic_trace(&self) -> Vec<String> {
        CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map_or_else(Vec::new, |c| c.trace.clone())
        })
    }
    pub fn poll(&self, allowed: bool) -> bool {
        let action = CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return Poll::None;
            };
            context.allowed = allowed;
            context
                .gesture
                .poll(context.clock.elapsed().as_millis() as u64, allowed)
        });
        if let Poll::Forward(ctrl) = action
            && !replay(&[ctrl])
        {
            let hwnd = CONTEXT.with(|cell| cell.borrow().as_ref().map_or(0, |c| c.hwnd));
            unsafe {
                PostMessageW(hwnd as HWND, CTRL_FAILED, 0, 0);
            }
        }
        action == Poll::Start
    }
}
impl Drop for CtrlHotkey {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.handle);
        }
        let pending = CONTEXT.with(|cell| cell.borrow_mut().take());
        if let Some(context) = pending
            && matches!(
                context.gesture.phase,
                Phase::Pending | Phase::Recording | Phase::Cancelled
            )
        {
            // Restore a down event so the eventual physical release stays balanced.
            let _ = replay(&[context.gesture.ctrl]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(vk: u16, up: bool) -> Key {
        Key {
            vk,
            up,
            scan: 0,
            extended: vk == VK_RCONTROL,
        }
    }
    #[test]
    fn starts_at_300_ms_once_and_releases() {
        let mut g = Gesture::default();
        assert_eq!(
            g.input(key(VK_LCONTROL, false), 100, true, false),
            Effect::Suppress
        );
        assert_eq!(g.poll(399, true), Poll::None);
        assert_eq!(g.poll(400, true), Poll::Start);
        assert_eq!(g.poll(1600, false), Poll::None);
        assert_eq!(
            g.input(key(VK_LCONTROL, true), 1700, false, false),
            Effect::Release
        );
        assert_eq!(g.phase, Phase::Idle);
    }
    #[test]
    fn short_ctrl_replays_native_tap_without_recording() {
        let mut g = Gesture::default();
        let ctrl = key(VK_LCONTROL, false);
        g.input(ctrl, 0, true, false);
        assert_eq!(
            g.input(key(VK_LCONTROL, true), 299, true, false),
            Effect::Tap(ctrl)
        );
        assert_eq!(g.poll(1000, true), Poll::None);
    }
    #[test]
    fn combos_cancel_pending_and_replay_ordered_ctrl() {
        for other in [VK_TAB, VK_F4, VK_SPACE, 0x41, 0x43, 0x56, 0x58, 0x5a] {
            let mut g = Gesture::default();
            let ctrl = key(VK_LCONTROL, false);
            let letter = key(other, false);
            g.input(ctrl, 0, true, false);
            assert_eq!(
                g.input(letter, 100, true, false),
                Effect::Combo(ctrl, letter, false)
            );
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(
                g.input(key(VK_LCONTROL, true), 1200, true, false),
                Effect::Pass
            );
        }
    }
    #[test]
    fn other_modifiers_and_unavailable_app_pass_through() {
        for ctrl in [VK_LCONTROL, VK_RCONTROL] {
            let mut g = Gesture::default();
            assert_eq!(g.input(key(ctrl, false), 0, true, true), Effect::Pass);
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(g.input(key(ctrl, false), 0, false, false), Effect::Pass);
        }
    }
    #[test]
    fn escape_cancels_and_combo_during_recording_restores_native_input() {
        let mut g = Gesture::default();
        let ctrl = key(VK_RCONTROL, false);
        g.input(ctrl, 0, true, false);
        assert_eq!(g.poll(300, true), Poll::Start);
        assert_eq!(
            g.input(key(VK_ESCAPE, false), 600, false, false),
            Effect::Cancel
        );
        assert_eq!(
            g.input(key(VK_ESCAPE, false), 650, false, false),
            Effect::Suppress
        );
        assert_eq!(
            g.input(key(VK_RCONTROL, true), 700, false, false),
            Effect::Suppress
        );
        g.input(ctrl, 1000, true, false);
        g.poll(1300, true);
        let tab = key(VK_TAB, false);
        assert_eq!(
            g.input(tab, 1600, false, false),
            Effect::Combo(ctrl, tab, true)
        );
    }
    #[test]
    fn pending_is_restored_if_settings_or_busy_state_blocks_start() {
        let mut g = Gesture::default();
        let ctrl = key(VK_LCONTROL, false);
        g.input(ctrl, 0, true, false);
        assert_eq!(g.poll(200, false), Poll::Forward(ctrl));
        assert_eq!(
            g.input(key(VK_LCONTROL, true), 800, false, false),
            Effect::Pass
        );
    }
}
