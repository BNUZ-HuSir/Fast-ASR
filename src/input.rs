use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*},
    UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static FOCUS_EPOCH: AtomicU64 = AtomicU64::new(0);
unsafe extern "system" fn focus_event(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    FOCUS_EPOCH.fetch_add(1, Ordering::Relaxed);
}
pub fn focus_hook() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            std::ptr::null_mut(),
            Some(focus_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub window: isize,
    pub focus: isize,
    epoch: u64,
}
impl Target {
    pub fn capture(exclude: HWND) -> Option<Self> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() || hwnd == exclude {
                return None;
            }
            let thread = GetWindowThreadProcessId(hwnd, null_mut());
            let mut info: GUITHREADINFO = zeroed();
            info.cbSize = size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(thread, &mut info) == 0 {
                return None;
            }
            if info.hwndFocus.is_null() {
                return None;
            }
            Some(Self {
                window: hwnd as isize,
                focus: info.hwndFocus as isize,
                epoch: FOCUS_EPOCH.load(Ordering::Relaxed),
            })
        }
    }
    pub fn matches(&self) -> bool {
        Self::capture(null_mut()).is_some_and(|t| {
            t.window == self.window && t.focus == self.focus && t.epoch == self.epoch
        })
    }
    /// Recover after returning to the exact same native input control.
    pub fn refresh_if_current(&mut self) -> bool {
        if let Some(now) = Self::capture(std::ptr::null_mut())
            && now.window == self.window
            && now.focus == self.focus
            && !now.password()
        {
            self.epoch = now.epoch;
            return true;
        }
        false
    }
    pub fn title(&self) -> String {
        let mut text = [0u16; 256];
        let n =
            unsafe { GetWindowTextW(self.window as HWND, text.as_mut_ptr(), text.len() as i32) };
        String::from_utf16_lossy(&text[..n.max(0) as usize])
    }
    pub fn password(&self) -> bool {
        unsafe {
            let hwnd = self.focus as HWND;
            let mut name = [0u16; 64];
            let n = GetClassNameW(hwnd, name.as_mut_ptr(), 64);
            String::from_utf16_lossy(&name[..n.max(0) as usize]).eq_ignore_ascii_case("Edit")
                && GetWindowLongPtrW(hwnd, GWL_STYLE) & 0x20 != 0
        }
    }
}
pub fn modifiers_down() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|&key| unsafe { GetAsyncKeyState(key as i32) < 0 })
}
pub fn type_text(target: Target, text: &str) -> Result<(), String> {
    if !target.matches() {
        return Err("输入焦点已变化，文字已保留。点击目标输入框后再点“填写”。".into());
    }
    if target.password() {
        return Err("当前为密码输入框，文字已保留。".into());
    }
    if modifiers_down() {
        return Err("请松开 Ctrl / Alt / Shift / Win 后重试".into());
    }
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for code in text.encode_utf16() {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(format!(
            "输入未完整发送（{sent}/{}）。可能是权限限制；文字已保留，重试前请检查是否已部分写入。",
            inputs.len()
        ));
    }
    Ok(())
}
pub fn copy_text(owner: HWND, text: &str) -> Result<(), String> {
    unsafe {
        if OpenClipboard(owner) == 0 {
            return Err("剪贴板正忙，请稍后再试".into());
        }
        let bytes = wide(text);
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len() * 2);
        if mem.is_null() {
            CloseClipboard();
            return Err("无法分配剪贴板内存".into());
        }
        let dest = GlobalLock(mem);
        if dest.is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return Err("无法访问剪贴板".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest.cast(), bytes.len());
        GlobalUnlock(mem);
        if EmptyClipboard() == 0 || SetClipboardData(13, mem).is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return Err("无法写入剪贴板".into());
        }
        CloseClipboard();
        Ok(())
    }
}
