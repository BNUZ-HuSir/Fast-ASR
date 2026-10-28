//! Delayed, standalone left Alt gesture. The hook never performs audio or inference work.
use std::{cell::RefCell, mem::size_of, ptr::null, ptr::null_mut, time::Instant};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub const HOLD_RELEASE: u32 = WM_APP + 20;
pub const HOLD_CANCEL: u32 = WM_APP + 21;
pub const HOLD_FAILED: u32 = WM_APP + 22;
const OWN_INPUT: usize = 0x46415352;
pub const TEST_INPUT: usize = 0x46415354;
const HOLD_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    vk: u16,
    scan: u16,
    extended: bool,
    up: bool,
}
impl Key {
    fn alt(self) -> bool {
        self.vk == VK_LMENU
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
    alt: Key,
    since: u64,
}
impl Default for Gesture {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            alt: Key {
                vk: VK_LMENU,
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
            if key.alt() && !key.up && allowed && !other_down {
                self.phase = Phase::Pending;
                self.alt = key;
                self.since = now;
                return Effect::Suppress;
            }
            return Effect::Pass;
        }
        if self.phase == Phase::Forwarding {
            if key.vk == self.alt.vk && key.up {
                self.phase = Phase::Idle;
            }
            return Effect::Pass;
        }
        if key.vk == self.alt.vk {
            if !key.up {
                return Effect::Suppress;
            }
            let previous = self.phase;
            self.phase = Phase::Idle;
            return match previous {
                Phase::Pending => Effect::Tap(self.alt),
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
            return Effect::Combo(self.alt, key, cancel);
        }
        Effect::Pass
    }
    fn mouse_chord(&mut self) -> Option<(Key, bool)> {
        if matches!(
            self.phase,
            Phase::Pending | Phase::Recording | Phase::Cancelled
        ) {
            let cancel = self.phase == Phase::Recording;
            self.phase = Phase::Forwarding;
            Some((self.alt, cancel))
        } else {
            None
        }
    }
    fn poll(&mut self, now: u64, allowed: bool) -> Poll {
        if self.phase != Phase::Pending {
            return Poll::None;
        }
        if !allowed {
            self.phase = Phase::Forwarding;
            return Poll::Forward(self.alt);
        }
        if now.saturating_sub(self.since) >= HOLD_MS {
            self.phase = Phase::Recording;
            return Poll::Start;
        }
        Poll::None
    }
}
// The UI, hook callbacks and input replay never synchronously wait on each other.
// In particular SendInput must not execute on the low-level hook thread.
#[derive(Clone, Copy, Debug)]
enum Replay {
    Key(Key),
    Mouse {
        flags: u32,
        data: u32,
        x: i32,
        y: i32,
    },
}
struct State {
    clock: Instant,
    gesture: Gesture,
    allowed: bool,
    enabled: bool,
    pending: usize,
    test_input: bool,
    trace: Vec<String>,
}
struct Shared {
    hwnd: isize,
    state: std::sync::Mutex<State>,
    replay: std::sync::mpsc::Sender<Vec<Replay>>,
}
thread_local! { static CONTEXT: RefCell<Option<std::sync::Arc<Shared>>> = const { RefCell::new(None) }; }
impl Shared {
    fn queue(&self, state: &mut State, events: Vec<Replay>) -> bool {
        state.pending += 1;
        if self.replay.send(events).is_ok() {
            return true;
        }
        state.pending -= 1;
        state.enabled = false;
        unsafe {
            PostMessageW(self.hwnd as HWND, HOLD_FAILED, 0, 0);
        }
        false
    }
}
fn replay(events: &[Replay]) -> bool {
    let inputs: Vec<_> = events
        .iter()
        .map(|event| match *event {
            Replay::Key(key) => INPUT {
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
            },
            Replay::Mouse { flags, data, x, y } => {
                // Preserve the location of delayed clicks across multiple monitors.
                let (left, top, width, height) = unsafe {
                    (
                        GetSystemMetrics(SM_XVIRTUALSCREEN),
                        GetSystemMetrics(SM_YVIRTUALSCREEN),
                        GetSystemMetrics(SM_CXVIRTUALSCREEN),
                        GetSystemMetrics(SM_CYVIRTUALSCREEN),
                    )
                };
                let absolute = |value: i32, origin: i32, span: i32| {
                    (((value as i64 - origin as i64) * 65536 + 32768) / span.max(1) as i64)
                        .clamp(0, 65535) as i32
                };
                INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: absolute(x, left, width),
                            dy: absolute(y, top, height),
                            mouseData: data,
                            dwFlags: flags
                                | MOUSEEVENTF_MOVE
                                | MOUSEEVENTF_ABSOLUTE
                                | MOUSEEVENTF_VIRTUALDESK,
                            time: 0,
                            dwExtraInfo: OWN_INPUT,
                        },
                    },
                }
            }
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
        let shared = CONTEXT.with(|cell| cell.borrow().clone());
        let Some(shared) = shared else {
            return CallNextHookEx(null_mut(), code, w, l);
        };
        // Own replay must pass immediately, without waiting for a state lock.
        if event.flags & LLKHF_INJECTED != 0 && event.dwExtraInfo == OWN_INPUT {
            return CallNextHookEx(null_mut(), code, w, l);
        }
        let suppress = {
            let Ok(mut state) = shared.state.lock() else {
                return CallNextHookEx(null_mut(), code, w, l);
            };
            if event.flags & LLKHF_INJECTED != 0
                && !(state.test_input && event.dwExtraInfo == TEST_INPUT)
            {
                false
            } else {
                let key = Key {
                    vk: event.vkCode as u16,
                    scan: event.scanCode as u16,
                    extended: event.flags & LLKHF_EXTENDED != 0,
                    up: event.flags & LLKHF_UP != 0,
                };
                let other_down = key.alt()
                    && (1..=254u16).any(|vk| {
                        ![VK_MENU, VK_LMENU].contains(&vk) && GetAsyncKeyState(vk as i32) < 0
                    });
                let now = state.clock.elapsed().as_millis() as u64;
                let allowed = state.allowed && state.enabled && state.pending == 0;
                let effect = state.gesture.input(key, now, allowed, other_down);
                if state.test_input {
                    state.trace.push(format!(
                        "key={key:?} flags={} other={other_down} effect={effect:?}",
                        event.flags
                    ));
                }
                match effect {
                    // Serialize physical key-up too while a replay batch is pending.
                    // Otherwise a quick release could overtake its delayed key-down.
                    Effect::Pass if state.pending > 0 => {
                        shared.queue(&mut state, vec![Replay::Key(key)])
                    }
                    Effect::Pass => false,
                    Effect::Suppress => true,
                    Effect::Tap(alt) => shared.queue(
                        &mut state,
                        vec![Replay::Key(alt), Replay::Key(Key { up: true, ..alt })],
                    ),
                    Effect::Combo(alt, key, cancel) => {
                        if cancel {
                            PostMessageW(shared.hwnd as HWND, HOLD_CANCEL, 0, 0);
                        }
                        shared.queue(&mut state, vec![Replay::Key(alt), Replay::Key(key)])
                    }
                    Effect::Release => {
                        PostMessageW(shared.hwnd as HWND, HOLD_RELEASE, 0, 0);
                        true
                    }
                    Effect::Cancel => {
                        PostMessageW(shared.hwnd as HWND, HOLD_CANCEL, 0, 0);
                        true
                    }
                }
            }
        };
        if suppress {
            1
        } else {
            CallNextHookEx(null_mut(), code, w, l)
        }
    }
}
fn mouse_action(message: u32, data: u32) -> Option<(u32, u32)> {
    Some(match message {
        WM_LBUTTONDOWN => (MOUSEEVENTF_LEFTDOWN, 0),
        WM_LBUTTONUP => (MOUSEEVENTF_LEFTUP, 0),
        WM_RBUTTONDOWN => (MOUSEEVENTF_RIGHTDOWN, 0),
        WM_RBUTTONUP => (MOUSEEVENTF_RIGHTUP, 0),
        WM_MBUTTONDOWN => (MOUSEEVENTF_MIDDLEDOWN, 0),
        WM_MBUTTONUP => (MOUSEEVENTF_MIDDLEUP, 0),
        WM_XBUTTONDOWN => (MOUSEEVENTF_XDOWN, data >> 16),
        WM_XBUTTONUP => (MOUSEEVENTF_XUP, data >> 16),
        WM_MOUSEWHEEL => (
            MOUSEEVENTF_WHEEL,
            ((data >> 16) as u16 as i16) as i32 as u32,
        ),
        WM_MOUSEHWHEEL => (
            MOUSEEVENTF_HWHEEL,
            ((data >> 16) as u16 as i16) as i32 as u32,
        ),
        _ => return None,
    })
}
fn initiates_mouse_chord(message: u32) -> bool {
    matches!(
        message,
        WM_LBUTTONDOWN
            | WM_RBUTTONDOWN
            | WM_MBUTTONDOWN
            | WM_XBUTTONDOWN
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
    )
}
unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(null_mut(), code, w, l);
        }
        let event = &*(l as *const MSLLHOOKSTRUCT);
        if event.flags & LLMHF_INJECTED != 0 && event.dwExtraInfo == OWN_INPUT {
            return CallNextHookEx(null_mut(), code, w, l);
        }
        let Some((flags, data)) = mouse_action(w as u32, event.mouseData) else {
            return CallNextHookEx(null_mut(), code, w, l);
        };
        let shared = CONTEXT.with(|cell| cell.borrow().clone());
        let Some(shared) = shared else {
            return CallNextHookEx(null_mut(), code, w, l);
        };
        let suppress = {
            let Ok(mut state) = shared.state.lock() else {
                return CallNextHookEx(null_mut(), code, w, l);
            };
            if event.flags & LLMHF_INJECTED != 0
                && !(state.test_input && event.dwExtraInfo == TEST_INPUT)
            {
                false
            } else {
                let action = if initiates_mouse_chord(w as u32) {
                    state.gesture.mouse_chord()
                } else {
                    None
                };
                if state.test_input {
                    state.trace.push(format!("mouse={w} effect={action:?}"));
                }
                let mut events = Vec::new();
                if let Some((alt, cancel)) = action {
                    if cancel {
                        PostMessageW(shared.hwnd as HWND, HOLD_CANCEL, 0, 0);
                    }
                    events.push(Replay::Key(alt));
                }
                if !events.is_empty() || state.pending > 0 {
                    events.push(Replay::Mouse {
                        flags,
                        data,
                        x: event.pt.x,
                        y: event.pt.y,
                    });
                    shared.queue(&mut state, events)
                } else {
                    false
                }
            }
        };
        if suppress {
            1
        } else {
            CallNextHookEx(null_mut(), code, w, l)
        }
    }
}

pub struct AltHotkey {
    shared: Option<std::sync::Arc<Shared>>,
    thread_id: std::sync::Arc<std::sync::atomic::AtomicU32>,
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
    hook_done: std::sync::mpsc::Receiver<Result<(), String>>,
    replay_done: std::sync::mpsc::Receiver<()>,
    hook_thread: Option<std::thread::JoinHandle<()>>,
    replay_thread: Option<std::thread::JoinHandle<()>>,
}
impl AltHotkey {
    pub fn install(hwnd: HWND, test_input: bool) -> Result<Self, String> {
        use std::sync::{Arc, Mutex, atomic::AtomicU32, mpsc};
        let (replay_tx, replay_rx) = mpsc::channel::<Vec<Replay>>();
        let shared = Arc::new(Shared {
            hwnd: hwnd as isize,
            replay: replay_tx,
            state: Mutex::new(State {
                clock: Instant::now(),
                gesture: Gesture::default(),
                allowed: false,
                enabled: true,
                pending: 0,
                test_input,
                trace: Vec::new(),
            }),
        });
        let weak = Arc::downgrade(&shared);
        let (replay_done_tx, replay_done) = mpsc::channel();
        let replay_thread = std::thread::Builder::new()
            .name("asr-input-replay".into())
            .spawn(move || {
                while let Ok(events) = replay_rx.recv() {
                    let success = replay(&events);
                    if let Some(shared) = weak.upgrade() {
                        if let Ok(mut state) = shared.state.lock() {
                            state.pending = state.pending.saturating_sub(1);
                            if !success {
                                state.enabled = false;
                            }
                        }
                        if !success {
                            unsafe {
                                PostMessageW(shared.hwnd as HWND, HOLD_FAILED, 0, 0);
                            }
                        }
                    }
                }
                let _ = replay_done_tx.send(());
            })
            .map_err(|e| e.to_string())?;
        let thread_id = Arc::new(AtomicU32::new(0));
        let hook_id = Arc::clone(&thread_id);
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hook_stopping = Arc::clone(&stopping);
        let hook_shared = Arc::clone(&shared);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, hook_done) = mpsc::channel();
        let hook_thread = std::thread::Builder::new()
            .name("asr-input-hook".into())
            .spawn(move || unsafe {
                use std::sync::atomic::Ordering;
                let mut msg = std::mem::zeroed::<MSG>();
                PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
                hook_id.store(
                    windows_sys::Win32::System::Threading::GetCurrentThreadId(),
                    Ordering::Release,
                );
                CONTEXT.with(|cell| *cell.borrow_mut() = Some(hook_shared));
                let keyboard_handle =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(null()), 0);
                let mouse_handle = if keyboard_handle.is_null() {
                    null_mut()
                } else {
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), GetModuleHandleW(null()), 0)
                };
                let installed = !keyboard_handle.is_null() && !mouse_handle.is_null();
                let error = if installed {
                    None
                } else {
                    Some(format!(
                        "无法启用左 Alt 快捷键（Windows {}），请使用麦克风按钮",
                        GetLastError()
                    ))
                };
                let _ = ready_tx.send(error.clone().map_or(Ok(()), Err));
                if installed && !hook_stopping.load(Ordering::Acquire) {
                    loop {
                        let result = GetMessageW(&mut msg, null_mut(), 0, 0);
                        if result <= 0 {
                            break;
                        }
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                let mut result = error.map_or(Ok(()), Err);
                for handle in [mouse_handle, keyboard_handle] {
                    if !handle.is_null() && UnhookWindowsHookEx(handle) == 0 {
                        result = Err(format!("输入监听卸载失败（Windows {}）", GetLastError()));
                    }
                }
                CONTEXT.with(|cell| *cell.borrow_mut() = None);
                let _ = done_tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        let mut hook = Self {
            shared: Some(shared),
            thread_id,
            stopping,
            hook_done,
            replay_done,
            hook_thread: Some(hook_thread),
            replay_thread: Some(replay_thread),
        };
        match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(hook),
            Ok(Err(error)) => {
                let _ = hook.shutdown();
                Err(error)
            }
            Err(error) => {
                let _ = hook.shutdown();
                Err(format!("输入监听启动超时：{error}"))
            }
        }
    }
    pub fn diagnostic_trace(&self) -> Vec<String> {
        self.shared
            .as_ref()
            .and_then(|s| s.state.lock().ok())
            .map_or_else(Vec::new, |s| s.trace.clone())
    }
    pub fn poll(&self, allowed: bool) -> bool {
        let Some(shared) = &self.shared else {
            return false;
        };
        let Ok(mut state) = shared.state.lock() else {
            return false;
        };
        state.allowed = allowed;
        let allowed = allowed && state.enabled;
        let now = state.clock.elapsed().as_millis() as u64;
        let action = state.gesture.poll(now, allowed);
        if let Poll::Forward(alt) = action {
            shared.queue(&mut state, vec![Replay::Key(alt)]);
        }
        action == Poll::Start
    }
    /// Bounded shutdown; diagnostics must report an incomplete cleanup as a failure.
    pub fn shutdown(&mut self) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        if self.hook_thread.is_none() {
            return Ok(());
        }
        if let Some(shared) = &self.shared
            && let Ok(mut state) = shared.state.lock()
        {
            state.allowed = false;
            if matches!(
                state.gesture.phase,
                Phase::Pending | Phase::Recording | Phase::Cancelled
            ) {
                let alt = state.gesture.alt;
                // A complete tap balances a suppressed down even if release races teardown.
                shared.queue(
                    &mut state,
                    vec![Replay::Key(alt), Replay::Key(Key { up: true, ..alt })],
                );
                state.gesture.phase = Phase::Idle;
            }
        }
        self.stopping.store(true, Ordering::Release);
        let id = self.thread_id.load(Ordering::Acquire);
        if id != 0 {
            unsafe {
                PostThreadMessageW(id, WM_QUIT, 0, 0);
            }
        }
        let hook_result = self
            .hook_done
            .recv_timeout(std::time::Duration::from_secs(1));
        self.shared = None;
        let replay_result = self
            .replay_done
            .recv_timeout(std::time::Duration::from_secs(1));
        let hook_thread = self.hook_thread.take();
        let replay_thread = self.replay_thread.take();
        if hook_result.is_ok()
            && let Some(thread) = hook_thread
        {
            let _ = thread.join();
        }
        if replay_result.is_ok()
            && let Some(thread) = replay_thread
        {
            let _ = thread.join();
        }
        hook_result.map_err(|_| "输入监听退出未完成".to_string())??;
        replay_result.map_err(|_| "输入转发退出未完成".to_string())?;
        Ok(())
    }
}
impl Drop for AltHotkey {
    fn drop(&mut self) {
        let _ = self.shutdown();
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
            extended: vk == VK_RMENU,
        }
    }
    #[test]
    fn starts_at_500_ms_once_and_releases() {
        let mut g = Gesture::default();
        assert_eq!(
            g.input(key(VK_LMENU, false), 100, true, false),
            Effect::Suppress
        );
        assert_eq!(g.poll(400, true), Poll::None);
        assert_eq!(g.poll(599, true), Poll::None);
        assert_eq!(g.poll(600, true), Poll::Start);
        assert_eq!(g.poll(1600, false), Poll::None);
        assert_eq!(
            g.input(key(VK_LMENU, true), 1700, false, false),
            Effect::Release
        );
        assert_eq!(g.phase, Phase::Idle);
    }
    #[test]
    fn short_alt_replays_native_tap_without_recording() {
        let mut g = Gesture::default();
        let alt = key(VK_LMENU, false);
        g.input(alt, 0, true, false);
        assert_eq!(
            g.input(key(VK_LMENU, true), 499, true, false),
            Effect::Tap(alt)
        );
        assert_eq!(g.poll(1000, true), Poll::None);
    }
    #[test]
    fn combos_cancel_pending_and_replay_ordered_alt() {
        for other in [
            VK_TAB, VK_F4, VK_SPACE, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_CONTROL, 0x41, 0x43, 0x56,
            0x58, 0x5a,
        ] {
            let mut g = Gesture::default();
            let alt = key(VK_LMENU, false);
            let letter = key(other, false);
            g.input(alt, 0, true, false);
            assert_eq!(
                g.input(letter, 400, true, false),
                Effect::Combo(alt, letter, false)
            );
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(
                g.input(key(VK_LMENU, true), 1200, true, false),
                Effect::Pass
            );
        }
    }
    #[test]
    fn other_modifiers_and_unavailable_app_pass_through() {
        {
            let alt = VK_LMENU;
            let mut g = Gesture::default();
            assert_eq!(g.input(key(alt, false), 0, true, true), Effect::Pass);
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(g.input(key(alt, false), 0, false, false), Effect::Pass);
        }
    }
    #[test]
    fn escape_cancels_and_combo_during_recording_restores_native_input() {
        let mut g = Gesture::default();
        let alt = key(VK_LMENU, false);
        g.input(alt, 0, true, false);
        assert_eq!(g.poll(500, true), Poll::Start);
        assert_eq!(
            g.input(key(VK_ESCAPE, false), 600, false, false),
            Effect::Cancel
        );
        assert_eq!(
            g.input(key(VK_ESCAPE, false), 650, false, false),
            Effect::Suppress
        );
        assert_eq!(
            g.input(key(VK_LMENU, true), 700, false, false),
            Effect::Suppress
        );
        g.input(alt, 1000, true, false);
        g.poll(1500, true);
        let tab = key(VK_TAB, false);
        assert_eq!(
            g.input(tab, 1600, false, false),
            Effect::Combo(alt, tab, true)
        );
    }
    #[test]
    fn pending_is_restored_if_settings_or_busy_state_blocks_start() {
        let mut g = Gesture::default();
        let alt = key(VK_LMENU, false);
        g.input(alt, 0, true, false);
        assert_eq!(g.poll(200, false), Poll::Forward(alt));
        assert_eq!(
            g.input(key(VK_LMENU, true), 800, false, false),
            Effect::Pass
        );
    }
    #[test]
    fn ctrl_and_right_alt_never_start_recording() {
        for vk in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_RMENU] {
            let mut g = Gesture::default();
            assert_eq!(g.input(key(vk, false), 0, true, false), Effect::Pass);
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(g.input(key(vk, true), 1100, true, false), Effect::Pass);
        }
    }
    #[test]
    fn mouse_chord_forwards_alt_and_cancels_only_active_recording() {
        for recording in [false, true] {
            let mut g = Gesture::default();
            let alt = key(VK_LMENU, false);
            g.input(alt, 0, true, false);
            if recording {
                assert_eq!(g.poll(500, true), Poll::Start);
            }
            assert_eq!(g.mouse_chord(), Some((alt, recording)));
            assert_eq!(g.mouse_chord(), None);
            assert_eq!(g.poll(1000, true), Poll::None);
            assert_eq!(
                g.input(key(VK_LMENU, true), 1100, true, false),
                Effect::Pass
            );
        }
        let mut g = Gesture::default();
        assert_eq!(g.mouse_chord(), None);
    }
    #[test]
    fn mouse_mapping_preserves_buttons_and_signed_wheel_data() {
        assert_eq!(
            mouse_action(WM_LBUTTONDOWN, 0),
            Some((MOUSEEVENTF_LEFTDOWN, 0))
        );
        assert_eq!(
            mouse_action(WM_XBUTTONDOWN, 2 << 16),
            Some((MOUSEEVENTF_XDOWN, 2))
        );
        assert_eq!(
            mouse_action(WM_MOUSEWHEEL, ((-120i16) as u16 as u32) << 16),
            Some((MOUSEEVENTF_WHEEL, (-120i32) as u32))
        );
        assert_eq!(
            mouse_action(WM_MOUSEHWHEEL, 120 << 16),
            Some((MOUSEEVENTF_HWHEEL, 120))
        );
        assert_eq!(mouse_action(WM_MOUSEMOVE, 0), None);
        assert_eq!(mouse_action(WM_LBUTTONUP, 0), Some((MOUSEEVENTF_LEFTUP, 0)));
        assert!(!initiates_mouse_chord(WM_LBUTTONUP));
    }

    #[test]
    fn native_alt_chords_cancel_held_draft_without_submitting() {
        for vk in [
            VK_TAB, VK_F4, VK_SPACE, VK_RETURN, VK_SHIFT, VK_CONTROL, 0x41,
        ] {
            let mut g = Gesture::default();
            let alt = key(VK_LMENU, false);
            g.input(alt, 0, true, false);
            g.poll(500, true);
            assert_eq!(
                g.input(key(vk, false), 500, true, false),
                Effect::Combo(alt, key(vk, false), true)
            );
            assert_eq!(g.input(key(VK_LMENU, true), 600, true, false), Effect::Pass);
        }
    }
}
