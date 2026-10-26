use crate::{
    assets::ModelKind,
    audio,
    config::Settings,
    hotkey::{self, AltHotkey},
    input::{self, Target, wide},
    worker::{Command, Event, Worker},
};
use std::{
    collections::VecDeque,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Accessibility::UnhookWinEvent, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
        WindowsAndMessaging::*,
    },
};

const WIDTH: i32 = 520;
const HEIGHT: i32 = 178;
const TRAY_MSG: u32 = WM_APP + 1;
const HOTKEY: i32 = 1;
const ESCAPE_HOTKEY: i32 = 2;
const BG: u32 = 0x00201711;
const PANEL: u32 = 0x00372b20;
const TEXT: u32 = 0x00f3f4f6;
const MUTED: u32 = 0x00b8a797;
const TEAL: u32 = 0x00b4d92d;
const AMBER: u32 = 0x006fc6ff;
const GREEN: u32 = 0x0066c522;
const RED: u32 = 0x005252ef;

pub struct App {
    hwnd: HWND,
    settings_hwnd: HWND,
    home: PathBuf,
    settings: Settings,
    worker: Worker,
    ready: bool,
    listening: bool,
    start_pending: bool,
    stop_pending: bool,
    hold_pressed: bool,
    busy: bool,
    target: Option<Target>,
    paused: bool,
    focus_paused: bool,
    discard_cancelled_text: bool,
    layout_height: i32,
    content_height: i32,
    scroll: i32,
    last_preview: String,
    pending: VecDeque<String>,
    transcript: String,
    partial: String,
    notice: String,
    level: f32,
    scale: f32,
    fonts: [HFONT; 4],
    icon: HICON,
    devices: Vec<(u32, String)>,
    mic_combo: HWND,
    model_combo: HWND,
    pause_combo: HWND,
    hotkey_mode_combo: HWND,
    vad_combo: HWND,
    min_speech_combo: HWND,
    hotkey_registered: bool,
    alt_hotkey: Option<AltHotkey>,
    probe_stage: Option<u32>,
    probe_benchmark: Option<serde_json::Value>,
}
impl App {
    fn toggle_topmost(&mut self) {
        let enabled = !self.settings.topmost;
        unsafe {
            if SetWindowPos(
                self.hwnd,
                if enabled {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
            {
                self.notice = "无法切换置顶状态，请重试".into();
                return;
            }
        }
        self.settings.topmost = enabled;
        self.notice = if enabled {
            "已置顶 · 悬浮窗保持在其他窗口上方"
        } else {
            "已取消置顶 · 可从托盘重新显示"
        }
        .into();
        if let Err(e) = self.settings.save(&self.home) {
            self.notice = format!("置顶已切换，但设置保存失败：{e}");
        }
    }
    fn status(&self) -> (&str, u32) {
        if !self.ready {
            return (
                if self.busy {
                    "准备中"
                } else {
                    "加载失败"
                },
                AMBER,
            );
        }
        if self.busy {
            return ("处理中", AMBER);
        }
        if !self.pending.is_empty() && self.paused {
            return ("待填写", AMBER);
        }
        if self.listening {
            return ("正在听写", TEAL);
        }
        ("就绪", TEAL)
    }
    fn toggle(&mut self) {
        if self.listening || self.start_pending {
            self.stop(false);
            return;
        }
        self.start(false);
    }
    fn hotkey_press(&mut self) {
        if !self.settings_hwnd.is_null() {
            return;
        }
        if !self.settings.hotkey_hold {
            self.toggle();
        }
    }
    fn release_hold(&mut self, chord_down: bool) {
        if self.hold_pressed && !chord_down {
            self.stop(false);
        }
    }
    fn start(&mut self, held: bool) {
        if self.busy {
            return;
        }
        if !self.ready {
            self.notice = "正在重新准备识别引擎…".into();
            self.busy = true;
            let _ = self.worker.tx.send(Command::Retry);
            return;
        }
        if self.listening {
            self.stop(false);
            return;
        }
        let target = Target::capture(self.hwnd);
        if target.is_some_and(|t| t.password()) {
            self.notice = "请先选择普通文字输入框".into();
            return;
        }
        self.focus_paused = false;
        self.target = target;
        self.paused = target.is_none() || !self.pending.is_empty();
        self.partial.clear();
        if self.pending.is_empty() {
            self.transcript.clear();
        }
        self.notice = if target.is_some() {
            "开始说话，停顿后自动填写".into()
        } else {
            "未找到输入焦点，识别文字会保留供复制".into()
        };
        if held && target.is_some() {
            self.notice = "按住说话，松开后识别并填写 · Esc 取消".into();
        }
        self.hold_pressed = held;
        self.start_pending = true;
        self.stop_pending = false;
        self.busy = true;
        let command = if held {
            Command::StartHeld(self.settings.clone())
        } else {
            Command::Start(self.settings.clone())
        };
        let _ = self.worker.tx.send(command);
    }
    fn stop(&mut self, cancel: bool) {
        // Cancellation must still work while the last utterance is being decoded.
        if !self.ready
            || !(self.listening || self.start_pending || cancel && self.busy)
            || self.stop_pending && !cancel
        {
            return;
        }
        self.hold_pressed = false;
        self.stop_pending = true;
        self.busy = true;
        self.discard_cancelled_text |= cancel;
        let _ = self.worker.tx.send(if cancel {
            Command::Cancel
        } else {
            Command::Stop
        });
    }
    fn flush(&mut self) {
        if self.pending.is_empty() || self.paused || input::modifiers_down() {
            return;
        }
        let Some(target) = self.target else {
            self.paused = true;
            return;
        };
        if !target.matches() {
            self.paused = true;
            self.focus_paused = true;
            self.notice = "输入位置已变化，点回原输入框可继续填写".into();
            return;
        }
        while let Some(text) = self.pending.front() {
            match input::type_text(target, text) {
                Ok(()) => {
                    self.pending.pop_front();
                    self.notice = "已填写 · 继续说话即可".into();
                }
                Err(e) => {
                    self.paused = true;
                    self.notice = e;
                    break;
                }
            }
        }
    }
    fn retry_input(&mut self) {
        self.target = Target::capture(self.hwnd).filter(|t| !t.password());
        self.paused = self.target.is_none();
        self.focus_paused = false;
        if self.paused {
            self.notice = "请先点击目标输入框，再点“填写”".into();
        } else if self.pending.is_empty() {
            self.notice = "已绑定当前输入框，下一句话将自动填写".into();
        } else {
            self.flush();
        }
    }
    fn clear_text(&mut self) {
        self.stop(true);
        self.pending.clear();
        self.transcript.clear();
        self.partial.clear();
        self.notice = "本次文字已清空".into();
    }
    fn copy(&mut self) {
        let text = if self.pending.is_empty() {
            self.transcript.clone()
        } else {
            self.pending.iter().cloned().collect::<String>()
        };
        if text.is_empty() {
            self.notice = "暂时没有可复制的文字".into();
            return;
        }
        self.notice = match input::copy_text(self.hwnd, &text) {
            Ok(()) => "文字已复制".into(),
            Err(e) => e,
        };
    }
    fn tick(&mut self) {
        let alt_allowed = self.settings.hotkey
            && self.settings.hotkey_hold
            && self.settings_hwnd.is_null()
            && self.ready
            && !self.busy
            && !self.listening
            && !self.hold_pressed;
        if self
            .alt_hotkey
            .as_ref()
            .is_some_and(|hook| hook.poll(alt_allowed))
        {
            self.start(true);
        }
        while let Ok(event) = self.worker.rx.try_recv() {
            match event {
                Event::Loading(s) => {
                    self.notice = s;
                }
                Event::Ready => {
                    self.ready = true;
                    self.busy = false;
                    self.notice = if self.settings.model.streaming() {
                        "先点击输入框，再点麦克风开始"
                    } else {
                        "Turbo CPU 识别较慢 · 点击麦克风开始"
                    }
                    .into();
                }
                Event::Started => {
                    self.start_pending = false;
                    self.listening = true;
                    self.busy = self.stop_pending;
                    unsafe {
                        RegisterHotKey(self.hwnd, ESCAPE_HOTKEY, MOD_NOREPEAT, VK_ESCAPE as u32);
                    }
                }
                Event::Stopped => {
                    self.start_pending = false;
                    self.stop_pending = false;
                    self.hold_pressed = false;
                    self.discard_cancelled_text = false;
                    unsafe {
                        UnregisterHotKey(self.hwnd, ESCAPE_HOTKEY);
                    }
                    self.listening = false;
                    self.busy = !self.ready;
                    self.level = 0.;
                    self.partial.clear();
                    if self.ready && self.pending.is_empty() {
                        self.notice = "已停止 · 点击麦克风继续".into();
                    }
                }
                Event::Partial(s) => {
                    if !self.discard_cancelled_text {
                        self.partial = s;
                    }
                }
                Event::Final(mut s) => {
                    if self.discard_cancelled_text {
                        continue;
                    }
                    self.partial.clear();
                    if s.chars().last().is_some_and(|c| c.is_ascii_alphanumeric()) {
                        s.push(' ');
                    }
                    self.transcript.push_str(&s);

                    let pending_chars: usize = self.pending.iter().map(|s| s.chars().count()).sum();
                    self.pending.push_back(s);
                    if pending_chars > 12000 {
                        self.stop(false);
                        self.paused = true;
                        self.notice = "待填写文字较多，已暂停录音，请先复制或填写".into();
                    }
                }
                Event::Captured(_) => {}
                Event::Level(level) => {
                    self.level = level;
                }
                Event::Benchmarked(report) => {
                    self.probe_benchmark = Some(report);
                }
                Event::LoadFailed(e) => {
                    self.start_pending = false;
                    self.stop_pending = false;
                    self.hold_pressed = false;
                    self.ready = false;
                    self.busy = false;
                    self.listening = false;
                    self.notice = e;
                }
                Event::Error(e) => {
                    self.start_pending = false;
                    self.stop_pending = false;
                    self.hold_pressed = false;
                    unsafe {
                        UnregisterHotKey(self.hwnd, ESCAPE_HOTKEY);
                    }
                    self.notice = e;
                    self.listening = false;
                    self.busy = false;
                    self.level = 0.;
                }
            }
        }
        if self.listening && !self.paused && self.target.is_some_and(|t| !t.matches()) {
            self.paused = true;
            self.focus_paused = true;
            self.notice = "输入位置已变化，点回原输入框即可继续".into();
        }
        if self.focus_paused && self.target.as_mut().is_some_and(|t| t.refresh_if_current()) {
            self.paused = false;
            self.focus_paused = false;
            self.notice = "已恢复原输入框 · 继续说话即可".into();
        }
        self.resize_to_text();
        self.flush();
        unsafe {
            InvalidateRect(self.hwnd, null(), 0);
        }
        if self.ready && !self.busy {
            match self.probe_stage {
                Some(0) => {
                    let _ = self.worker.tx.send(Command::Benchmark);
                    self.probe_stage = Some(1);
                }
                Some(1) if self.probe_benchmark.is_some() => {
                    let _ = std::fs::create_dir_all(self.home.join("reports"));
                    unsafe {
                        self.save_overlay(&self.home.join("reports/overlay.bmp"));
                    }
                    self.open_settings();
                    self.probe_stage = Some(2);
                }
                Some(2) => unsafe {
                    let _ =
                        save_window(self.settings_hwnd, &self.home.join("reports/settings.bmp"));
                    let noactivate =
                        SendMessageW(self.hwnd, WM_MOUSEACTIVATE, 0, 0) == MA_NOACTIVATE as isize;
                    let settings_opened = !self.settings_hwnd.is_null();
                    // Keep the foreground settings window alive while the test host
                    // is activated; destroying it first hands focus to another app.
                    let regression = self.regression_probe();
                    DestroyWindow(self.settings_hwnd);
                    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":crate::diagnostics::executable_sha256().unwrap_or_default(),"passed":noactivate&&settings_opened&&regression["passed"]==true&&self.probe_benchmark.as_ref().is_some_and(|v|v["passed"]==true),"non_activating_overlay":noactivate,"settings_opened":settings_opened,"regressions":regression,"memory_with_settings":crate::diagnostics::memory(),"recognition_sample":self.probe_benchmark});
                    let _ = std::fs::write(
                        self.home.join("reports/ui-probe.json"),
                        serde_json::to_vec_pretty(&report).unwrap(),
                    );
                    self.probe_stage = Some(3);
                },
                Some(3) => unsafe {
                    DestroyWindow(self.hwnd);
                    self.probe_stage = None;
                },
                _ => {}
            }
        }
    }
    fn regression_probe(&mut self) -> serde_json::Value {
        unsafe {
            fn pump_test_messages() {
                unsafe {
                    for _ in 0..30 {
                        let mut msg: MSG = zeroed();
                        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                            // Activation messages must run before checking focus. Avoid
                            // reentering tick() while this synchronous probe is active.
                            if msg.message != WM_TIMER {
                                TranslateMessage(&msg);
                                DispatchMessageW(&msg);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            }
            fn value(hwnd: HWND) -> String {
                unsafe {
                    let mut buf = [0u16; 1024];
                    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 1024);
                    String::from_utf16_lossy(&buf[..n.max(0) as usize])
                }
            }
            let instance = GetModuleHandleW(null());
            let host = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Fast ASR · 输入回归测试").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                600,
                220,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                20,
                20,
                550,
                40,
                host,
                null_mut(),
                instance,
                null(),
            );
            let second = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                20,
                80,
                550,
                40,
                host,
                null_mut(),
                instance,
                null(),
            );
            ShowWindow(host, SW_SHOW);
            crate::diagnostics::focus_probe_window(host, edit);
            pump_test_messages();
            self.probe_stage = Some(98);
            self.listening = true;
            self.start_pending = false;
            self.stop_pending = false;
            self.busy = false;
            self.target = Target::capture(self.hwnd);
            self.paused = false;
            self.focus_paused = false;
            if self
                .target
                .is_none_or(|t| t.window != host as isize || t.focus != edit as isize)
            {
                let details = serde_json::json!({"passed":false,"error":"Test focus unavailable", "host":host as usize,"edit":edit as usize,"foreground":GetForegroundWindow() as usize,"focus":GetFocus() as usize,"target":self.target.map(|t|format!("{t:?}"))});
                DestroyWindow(host);
                return details;
            }
            let initial_topmost = self.settings.topmost;
            let pin_click = (((23.0 * self.scale).round() as i32) << 16)
                | ((372.0 * self.scale).round() as i32);
            SendMessageW(self.hwnd, WM_LBUTTONUP, 0, pin_click as isize);
            let pin_toggled = (GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize
                != 0)
                != initial_topmost;
            let pin_kept_focus = GetForegroundWindow() == host && GetFocus() == edit;
            let pin_persisted = Settings::load(&self.home).topmost != initial_topmost;
            SendMessageW(self.hwnd, WM_LBUTTONUP, 0, pin_click as isize);
            let pin_restored = (GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize
                != 0)
                == initial_topmost
                && Settings::load(&self.home).topmost == initial_topmost;
            self.transcript = "复制回归测试".into();
            self.pending.clear();
            self.copy();
            let copied = self.notice == "文字已复制";
            self.pending.push_back("复制后自动填写".into());
            self.tick();
            pump_test_messages();
            let copy_resumed = value(edit) == "复制后自动填写" && self.pending.is_empty();
            SetFocus(second);
            self.pending.push_back("回到原框后填写".into());
            self.tick();
            pump_test_messages();
            let guarded = value(second).is_empty() && self.pending.len() == 1;
            SetFocus(edit);
            self.tick();
            pump_test_messages();
            let return_resumed =
                value(edit) == "复制后自动填写回到原框后填写" && self.pending.is_empty();
            self.target = None;
            self.paused = true;
            self.retry_input();
            self.pending.push_back("主动绑定成功".into());
            self.tick();
            pump_test_messages();
            let rearmed = value(edit).ends_with("主动绑定成功");
            self.listening = false;
            self.busy = true;
            self.clear_text();
            let cancellation_sent = self.discard_cancelled_text;
            let cancel_start = std::time::Instant::now();
            while self.busy && cancel_start.elapsed() < std::time::Duration::from_secs(2) {
                self.tick();
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            let cancelled_processing = cancellation_sent
                && !self.busy
                && !self.discard_cancelled_text
                && self.transcript.is_empty();
            self.listening = false;
            self.target = None;
            self.pending.clear();
            self.paused = false;
            self.transcript="这是一段用于验证长文本的内容。窗口应当自动增高，文字完整保留，超过屏幕高度后可以滚动查看。English text remains readable.\n".repeat(40);
            self.resize_to_text();
            let grown = self.layout_height > HEIGHT;
            let scroll_max = (self.content_height - (self.layout_height - 124)).max(0);
            let full_text = self.last_preview.chars().count() > 2000;
            self.save_overlay(&self.home.join("reports/overlay-long-bottom.bmp"));
            self.scroll = 0;
            self.save_overlay(&self.home.join("reports/overlay-long-top.bmp"));
            let long = serde_json::json!({"grown":grown,"height":self.layout_height,"content_height":self.content_height,"scroll_max":scroll_max,"characters":self.last_preview.chars().count(),"full_text_retained":full_text});
            self.transcript.clear();
            self.resize_to_text();
            let shrunk = self.layout_height == HEIGHT;
            DestroyWindow(host);
            serde_json::json!({"passed":cancelled_processing&&pin_toggled&&pin_kept_focus&&pin_persisted&&pin_restored&&copied&&copy_resumed&&guarded&&return_resumed&&rearmed&&grown&&scroll_max>0&&full_text&&shrunk,"cancel_while_processing":cancelled_processing,"topmost_button":{"toggles":pin_toggled,"keeps_input_focus":pin_kept_focus,"persists":pin_persisted,"restores":pin_restored},"copy_succeeded":copied,"copy_then_auto_fill":copy_resumed,"other_input_protected":guarded,"return_to_original_resumes":return_resumed,"bind_without_pending_text":rearmed,"long_text":long,"shrinks_when_cleared":shrunk})
        }
    }
    fn set_hotkey(&mut self) {
        self.release_hold(false);
        self.alt_hotkey = None;
        unsafe {
            if self.hotkey_registered {
                UnregisterHotKey(self.hwnd, HOTKEY);
                self.hotkey_registered = false;
            }
            if self.settings.hotkey && self.settings.hotkey_hold {
                match AltHotkey::install(self.hwnd, false) {
                    Ok(hook) => {
                        self.alt_hotkey = Some(hook);
                    }
                    Err(e) => {
                        self.settings.hotkey = false;
                        self.notice = e;
                    }
                }
            } else if self.settings.hotkey {
                self.hotkey_registered = RegisterHotKey(
                    self.hwnd,
                    HOTKEY,
                    MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                    VK_SPACE as u32,
                ) != 0;
                if !self.hotkey_registered {
                    self.settings.hotkey = false;
                    self.notice = "Ctrl+Alt+空格已被占用，请使用麦克风按钮".into();
                }
            }
        }
    }
    fn open_settings(&mut self) {
        self.stop(false);
        unsafe {
            if !self.settings_hwnd.is_null() {
                ShowWindow(self.settings_hwnd, SW_SHOW);
                SetForegroundWindow(self.settings_hwnd);
                return;
            }
            let scale = self.scale;
            let size = |n: i32| (n as f32 * scale).round() as i32;
            let instance = GetModuleHandleW(null());
            let hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME,
                wide("FastAsrSettings").as_ptr(),
                wide("Fast ASR · 设置").as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                size(480),
                size(630),
                self.hwnd,
                null_mut(),
                instance,
                self as *mut _ as _,
            );
            self.settings_hwnd = hwnd;
            let child =
                |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| {
                    let control = CreateWindowExW(
                        0,
                        wide(class).as_ptr(),
                        wide(text).as_ptr(),
                        WS_CHILD | WS_VISIBLE | style,
                        size(x),
                        size(y),
                        size(w),
                        size(h),
                        hwnd,
                        id as HMENU,
                        instance,
                        null(),
                    );
                    SendMessageW(control, WM_SETFONT, self.fonts[3] as usize, 1);
                    control
                };
            child("STATIC", "识别模型", 0, 24, 22, 100, 24, 0);
            self.model_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                24,
                48,
                390,
                180,
                100,
            );
            for kind in ModelKind::ALL {
                SendMessageW(
                    self.model_combo,
                    CB_ADDSTRING,
                    0,
                    wide(kind.label()).as_ptr() as isize,
                );
                if kind == self.settings.model {
                    SendMessageW(self.model_combo, CB_SETCURSEL, kind as usize, 0);
                }
            }
            child("STATIC", "麦克风", 0, 24, 90, 100, 24, 0);
            self.mic_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
                24,
                116,
                390,
                180,
                101,
            );
            self.devices = audio::devices();
            let mut selection = 0;
            for (i, (id, name)) in self.devices.iter().enumerate() {
                SendMessageW(
                    self.mic_combo,
                    CB_ADDSTRING,
                    0,
                    wide(name).as_ptr() as isize,
                );
                if *id == self.settings.microphone {
                    selection = i;
                }
            }
            SendMessageW(self.mic_combo, CB_SETCURSEL, selection, 0);
            child("STATIC", "停顿多久后填写", 0, 24, 158, 180, 24, 0);
            self.pause_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                24,
                184,
                390,
                130,
                102,
            );
            for label in ["快速 · 400 毫秒", "标准 · 600 毫秒", "宽松 · 1000 毫秒"] {
                SendMessageW(
                    self.pause_combo,
                    CB_ADDSTRING,
                    0,
                    wide(label).as_ptr() as isize,
                );
            }
            let selection = if self.settings.silence_ms <= 450 {
                0
            } else if self.settings.silence_ms >= 900 {
                2
            } else {
                1
            };
            SendMessageW(self.pause_combo, CB_SETCURSEL, selection, 0);
            child(
                "STATIC",
                "VAD 语音阈值 · 越低越敏感",
                0,
                24,
                224,
                400,
                24,
                0,
            );
            self.vad_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                24,
                250,
                390,
                200,
                105,
            );
            for label in [
                "0.2 · 最敏感",
                "0.3 · 高灵敏度",
                "0.4 · 较敏感",
                "0.5 · 标准（推荐）",
                "0.6 · 较严格",
                "0.7 · 严格",
                "0.8 · 最严格",
            ] {
                SendMessageW(
                    self.vad_combo,
                    CB_ADDSTRING,
                    0,
                    wide(label).as_ptr() as isize,
                );
            }
            SendMessageW(
                self.vad_combo,
                CB_SETCURSEL,
                ((self.settings.vad_threshold * 10.).round() as usize)
                    .saturating_sub(2)
                    .min(6),
                0,
            );
            child(
                "STATIC",
                "VAD 最短语音 · 过滤短促杂音",
                0,
                24,
                292,
                400,
                24,
                0,
            );
            self.min_speech_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                24,
                318,
                390,
                160,
                106,
            );
            for label in [
                "64 毫秒 · 短词更容易触发",
                "128 毫秒 · 标准（推荐）",
                "256 毫秒 · 过滤更多短音",
                "512 毫秒 · 严格过滤",
            ] {
                SendMessageW(
                    self.min_speech_combo,
                    CB_ADDSTRING,
                    0,
                    wide(label).as_ptr() as isize,
                );
            }
            let selection = match self.settings.vad_min_speech_ms {
                0..=64 => 0,
                65..=128 => 1,
                129..=256 => 2,
                _ => 3,
            };
            SendMessageW(self.min_speech_combo, CB_SETCURSEL, selection, 0);
            child("STATIC", "全局快捷键", 0, 24, 360, 400, 24, 0);
            self.hotkey_mode_combo = child(
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                24,
                386,
                390,
                160,
                103,
            );
            for label in [
                "关闭 · 使用麦克风按钮",
                "Alt 长按 0.5 秒录音 · 松开后识别",
                "Ctrl+Alt+空格 · 按一次开始/停止",
            ] {
                SendMessageW(
                    self.hotkey_mode_combo,
                    CB_ADDSTRING,
                    0,
                    wide(label).as_ptr() as isize,
                );
            }
            let selection = if !self.settings.hotkey {
                0
            } else if self.settings.hotkey_hold {
                1
            } else {
                2
            };
            SendMessageW(self.hotkey_mode_combo, CB_SETCURSEL, selection, 0);
            child(
                "STATIC",
                "两种模型均使用 Silero VAD，鼠标启动时停顿后填写。\n单独按住 Alt 满 0.5 秒开始，松开识别，最长 60 秒。\nEsc 取消；Turbo 在 CPU 上识别较慢。",
                0,
                24,
                430,
                420,
                72,
                0,
            );
            child(
                "BUTTON",
                "保存",
                BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
                314,
                520,
                100,
                32,
                104,
            );
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }
    fn save_settings(&mut self) {
        let previous_model = self.settings.model;
        unsafe {
            let model = SendMessageW(self.model_combo, CB_GETCURSEL, 0, 0);
            self.settings.model = ModelKind::ALL
                .get(model.max(0) as usize)
                .copied()
                .unwrap_or(ModelKind::XAsr480);
            let mic = SendMessageW(self.mic_combo, CB_GETCURSEL, 0, 0);
            if mic >= 0
                && let Some((id, _)) = self.devices.get(mic as usize)
            {
                self.settings.microphone = *id;
            }
            self.settings.silence_ms = match SendMessageW(self.pause_combo, CB_GETCURSEL, 0, 0) {
                0 => 400,
                2 => 1000,
                _ => 600,
            };
            let mode = SendMessageW(self.hotkey_mode_combo, CB_GETCURSEL, 0, 0);
            self.settings.hotkey = mode > 0;
            self.settings.hotkey_hold = mode == 1;
            self.settings.vad_threshold =
                (SendMessageW(self.vad_combo, CB_GETCURSEL, 0, 0).clamp(0, 6) as f32 + 2.) / 10.;
            self.settings.vad_min_speech_ms =
                match SendMessageW(self.min_speech_combo, CB_GETCURSEL, 0, 0) {
                    0 => 64,
                    2 => 256,
                    3 => 512,
                    _ => 128,
                };
            self.settings.normalize();
        }
        self.set_hotkey();
        match self.settings.save(&self.home) {
            Ok(()) => {
                self.notice = "设置已保存".into();
                if self.settings.model != previous_model {
                    self.switch_model(self.settings.model);
                }
                unsafe {
                    DestroyWindow(self.settings_hwnd);
                }
            }
            Err(e) => unsafe {
                MessageBoxW(
                    self.settings_hwnd,
                    wide(&format!("无法保存设置：{e}")).as_ptr(),
                    wide("Fast ASR").as_ptr(),
                    MB_ICONERROR,
                );
            },
        }
    }
    fn tray_menu(&mut self) {
        unsafe {
            let menu = CreatePopupMenu();
            for (id, label) in [
                (1, "显示悬浮条"),
                (
                    2,
                    if self.listening {
                        "停止听写"
                    } else {
                        "开始听写"
                    },
                ),
                (3, "设置"),
                (4, "退出"),
            ] {
                AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
            }
            let mut point: POINT = zeroed();
            GetCursorPos(&mut point);
            let old = GetForegroundWindow();
            SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                self.hwnd,
                null(),
            ) as u32;
            DestroyMenu(menu);
            if !old.is_null() && old != self.hwnd {
                SetForegroundWindow(old);
            }
            match command {
                1 => {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                2 => {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    self.toggle();
                }
                3 => self.open_settings(),
                4 => {
                    DestroyWindow(self.hwnd);
                }
                _ => {}
            }
        }
    }
    fn paint(&self) {
        unsafe {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(self.hwnd, &mut ps);
            let mut client: RECT = zeroed();
            GetClientRect(self.hwnd, &mut client);
            self.draw(dc, client.right, client.bottom);
            EndPaint(self.hwnd, &ps);
        }
    }
    unsafe fn save_overlay(&self, path: &std::path::Path) {
        unsafe {
            let dc = GetDC(self.hwnd);
            let mut client: RECT = zeroed();
            GetClientRect(self.hwnd, &mut client);
            let memory = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
            let old = SelectObject(memory, bitmap);
            self.draw(memory, client.right, client.bottom);
            let _ = write_bitmap(memory, bitmap, client.right, client.bottom, path);
            SelectObject(memory, old);
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(self.hwnd, dc);
        }
    }
    fn preview(&self) -> String {
        if !self.transcript.is_empty() {
            return format!("{}{}", self.transcript, self.partial);
        }
        if !self.partial.is_empty() {
            return self.partial.clone();
        }
        if !self.ready && !self.busy {
            "加载失败，点击麦克风重试".into()
        } else if self.listening {
            "正在听，请说话…".into()
        } else {
            "点击麦克风，开始语音输入".into()
        }
    }
    fn resize_to_text(&mut self) {
        let preview = self.preview();
        if preview == self.last_preview {
            return;
        }
        self.last_preview = preview.clone();
        unsafe {
            let dc = GetDC(self.hwnd);
            let old = SelectObject(dc, self.fonts[0]);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 401,
                bottom: 0,
            };
            let value = wide(&preview);
            DrawTextW(
                dc,
                value.as_ptr(),
                (value.len() - 1) as i32,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            ReleaseDC(self.hwnd, dc);
            self.content_height = (rect.bottom - rect.top).max(54);
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST),
                &mut info,
            );
            let max_height = (((info.rcWork.bottom - info.rcWork.top - 40) as f32 / self.scale)
                as i32)
                .clamp(HEIGHT, 720);
            let desired = (self.content_height + 124).clamp(HEIGHT, max_height);
            self.scroll = (self.content_height - (desired - 124)).max(0);
            if desired != self.layout_height {
                self.layout_height = desired;
                let height = (desired as f32 * self.scale).round() as i32;
                let width = (WIDTH as f32 * self.scale).round() as i32;
                let mut current: RECT = zeroed();
                GetWindowRect(self.hwnd, &mut current);
                let y = current
                    .top
                    .min(info.rcWork.bottom - height - 12)
                    .max(info.rcWork.top);
                SetWindowPos(
                    self.hwnd,
                    null_mut(),
                    current.left,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                SetWindowRgn(
                    self.hwnd,
                    CreateRoundRectRgn(
                        0,
                        0,
                        width + 1,
                        height + 1,
                        (18. * self.scale) as i32,
                        (18. * self.scale) as i32,
                    ),
                    1,
                );
            }
        }
    }
    fn switch_model(&mut self, kind: ModelKind) {
        self.hold_pressed = false;
        self.start_pending = false;
        self.stop_pending = false;
        self.settings.model = kind;
        let _ = self.settings.save(&self.home);
        self.ready = false;
        self.busy = true;
        self.listening = false;
        self.level = 0.;
        unsafe {
            UnregisterHotKey(self.hwnd, ESCAPE_HOTKEY);
        }
        self.notice = format!("正在切换 {}…", kind.label());
        let _ = self.worker.tx.send(Command::Switch(kind));
    }
    fn model_menu(&mut self) {
        if self.busy {
            return;
        }
        unsafe {
            let menu = CreatePopupMenu();
            for (i, kind) in ModelKind::ALL.into_iter().enumerate() {
                AppendMenuW(
                    menu,
                    MF_STRING
                        | if kind == self.settings.model {
                            MF_CHECKED
                        } else {
                            0
                        },
                    i + 1,
                    wide(kind.label()).as_ptr(),
                );
            }
            let mut p: POINT = zeroed();
            GetCursorPos(&mut p);
            let id = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                p.x,
                p.y,
                0,
                self.hwnd,
                null(),
            ) as usize;
            DestroyMenu(menu);
            if let Some(kind) = id
                .checked_sub(1)
                .and_then(|i| ModelKind::ALL.get(i))
                .copied()
                && kind != self.settings.model
            {
                self.switch_model(kind);
            }
        }
    }
    fn draw(&self, dc: HDC, width: i32, height: i32) {
        unsafe {
            let memory = CreateCompatibleDC(dc);
            let bmp = CreateCompatibleBitmap(dc, width, height);
            let old = SelectObject(memory, bmp);
            SetMapMode(memory, MM_ANISOTROPIC);
            SetWindowExtEx(memory, WIDTH, self.layout_height, null_mut());
            SetViewportExtEx(memory, width, height, null_mut());
            fill(
                memory,
                RECT {
                    left: 0,
                    top: 0,
                    right: WIDTH,
                    bottom: self.layout_height,
                },
                BG,
            );
            SetBkMode(memory, TRANSPARENT as i32);
            text(
                memory,
                self.fonts[1],
                "Fast ASR",
                TEXT,
                18,
                12,
                110,
                32,
                DT_SINGLELINE,
            );
            let (status, color) = self.status();
            text(
                memory,
                self.fonts[2],
                status,
                color,
                114,
                14,
                225,
                32,
                DT_SINGLELINE,
            );
            let model = match self.settings.model {
                ModelKind::XAsr480 => "X-ASR · 流式 ▾",
                ModelKind::WhisperTurboQ5 => "Turbo Q5 ▾",
            };
            text(
                memory,
                self.fonts[2],
                model,
                MUTED,
                230,
                14,
                340,
                32,
                DT_SINGLELINE,
            );
            fill(
                memory,
                RECT {
                    left: 344,
                    top: 9,
                    right: 400,
                    bottom: 35,
                },
                if self.settings.topmost { PANEL } else { BG },
            );
            draw_pin(memory, self.settings.topmost);
            text(
                memory,
                self.fonts[2],
                "设置",
                MUTED,
                407,
                14,
                450,
                32,
                DT_SINGLELINE,
            );
            text(
                memory,
                self.fonts[1],
                "−",
                MUTED,
                468,
                10,
                495,
                34,
                DT_CENTER | DT_SINGLELINE,
            );
            let recording = (self.listening || self.start_pending) && !self.stop_pending;
            let accent = if recording {
                RED
            } else if self.busy || !self.ready {
                PANEL
            } else {
                GREEN
            };
            let brush = CreateSolidBrush(accent);
            let previous = SelectObject(memory, brush);
            let pen = CreatePen(PS_SOLID, 1, accent);
            let previous_pen = SelectObject(memory, pen);
            Ellipse(memory, 18, 48, 82, 112);
            SelectObject(memory, previous);
            SelectObject(memory, previous_pen);
            DeleteObject(brush);
            DeleteObject(pen);
            draw_record_icon(
                memory,
                recording,
                self.busy && !self.start_pending,
                if self.busy && !recording { AMBER } else { TEXT },
            );
            let footer = self.layout_height - 70;
            let saved = SaveDC(memory);
            IntersectClipRect(memory, 98, 48, 500, footer - 6);
            text(
                memory,
                self.fonts[0],
                &self.preview(),
                TEXT,
                98,
                48 - self.scroll,
                499,
                48 - self.scroll + self.content_height + 4,
                DT_WORDBREAK,
            );
            RestoreDC(memory, saved);
            if self.content_height > footer - 54 {
                let track = (footer - 54).max(1);
                let visible = track * track / self.content_height;
                let top =
                    48 + (track - visible) * self.scroll / (self.content_height - track).max(1);
                fill(
                    memory,
                    RECT {
                        left: 505,
                        top,
                        right: 508,
                        bottom: top + visible.max(12),
                    },
                    MUTED,
                );
            }
            let subtitle = if self.hold_pressed {
                "按住录音 · 松开后识别"
            } else if self.listening {
                "正在录音 · 停顿后填写"
            } else {
                "中 / EN · 本地识别"
            };
            text(
                memory,
                self.fonts[2],
                subtitle,
                MUTED,
                98,
                footer,
                290,
                footer + 24,
                DT_SINGLELINE,
            );
            for (label, left, right, color) in [
                ("清空", 300, 343, MUTED),
                ("复制", 371, 414, MUTED),
                ("填写", 445, 491, TEAL),
            ] {
                text(
                    memory,
                    self.fonts[2],
                    label,
                    color,
                    left,
                    footer,
                    right,
                    footer + 24,
                    DT_SINGLELINE,
                );
            }
            fill(
                memory,
                RECT {
                    left: 18,
                    top: self.layout_height - 38,
                    right: 502,
                    bottom: self.layout_height - 37,
                },
                PANEL,
            );
            text(
                memory,
                self.fonts[2],
                &self.notice,
                if self.paused { AMBER } else { MUTED },
                18,
                self.layout_height - 29,
                502,
                self.layout_height - 6,
                DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            SetMapMode(memory, MM_TEXT);
            BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
            SelectObject(memory, old);
            DeleteObject(bmp);
            DeleteDC(memory);
        }
    }
}

unsafe fn draw_pin(dc: HDC, pinned: bool) {
    unsafe {
        let color = if pinned { TEAL } else { MUTED };
        let pen = CreatePen(PS_SOLID, 2, color);
        let old_pen = SelectObject(dc, pen);
        let brush = CreateSolidBrush(if pinned { color } else { BG });
        let old_brush = SelectObject(dc, brush);
        let points = [
            POINT { x: 366, y: 13 },
            POINT { x: 378, y: 13 },
            POINT { x: 376, y: 19 },
            POINT { x: 381, y: 24 },
            POINT { x: 363, y: 24 },
            POINT { x: 368, y: 19 },
        ];
        Polygon(dc, points.as_ptr(), points.len() as i32);
        MoveToEx(dc, 372, 24, null_mut());
        LineTo(dc, 372, 31);
        if !pinned {
            MoveToEx(dc, 361, 12, null_mut());
            LineTo(dc, 383, 32);
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}
unsafe fn draw_record_icon(dc: HDC, recording: bool, processing: bool, color: u32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 2, color);
        let old_pen = SelectObject(dc, pen);
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(dc, brush);
        if recording {
            RoundRect(dc, 40, 70, 61, 91, 5, 5);
        } else if processing {
            for x in [37, 48, 59] {
                Ellipse(dc, x, 78, x + 5, 83);
            }
        } else {
            RoundRect(dc, 45, 62, 56, 82, 10, 10);
            SelectObject(dc, GetStockObject(NULL_BRUSH));
            Arc(dc, 39, 69, 62, 91, 39, 78, 62, 78);
            MoveToEx(dc, 50, 90, null_mut());
            LineTo(dc, 50, 97);
            MoveToEx(dc, 44, 97, null_mut());
            LineTo(dc, 57, 97);
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}
unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn text(
    dc: HDC,
    font: HFONT,
    s: &str,
    color: u32,
    x: i32,
    y: i32,
    right: i32,
    bottom: i32,
    flags: u32,
) {
    unsafe {
        let previous = SelectObject(dc, font);
        SetTextColor(dc, color);
        let w = wide(s);
        let mut rect = RECT {
            left: x,
            top: y,
            right,
            bottom,
        };
        DrawTextW(
            dc,
            w.as_ptr(),
            (w.len() - 1) as i32,
            &mut rect,
            flags | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        let app = &mut *ptr;
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                app.paint();
                0
            }
            WM_TIMER => {
                app.tick();
                0
            }
            hotkey::ALT_FAILED => {
                app.stop(true);
                app.settings.hotkey = false;
                app.set_hotkey();
                let _ = app.settings.save(&app.home);
                app.notice = "Alt 键盘转发受限，快捷键已关闭，请使用麦克风按钮".into();
                0
            }
            hotkey::ALT_RELEASE => {
                app.release_hold(false);
                0
            }
            hotkey::ALT_CANCEL => {
                app.stop(true);
                0
            }
            WM_HOTKEY => {
                if w == HOTKEY as usize {
                    app.hotkey_press();
                } else if w == ESCAPE_HOTKEY as usize {
                    app.stop(true);
                }
                0
            }
            WM_NCHITTEST => {
                let mut p = POINT {
                    x: (l as i16) as i32,
                    y: ((l >> 16) as i16) as i32,
                };
                ScreenToClient(hwnd, &mut p);
                let x = (p.x as f32 / app.scale) as i32;
                let y = (p.y as f32 / app.scale) as i32;
                if y < 37 && x < 220 {
                    HTCAPTION as isize
                } else {
                    HTCLIENT as isize
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((w >> 16) as u16 as i16) as i32;
                app.scroll = (app.scroll - delta / 120 * 54)
                    .clamp(0, (app.content_height - (app.layout_height - 124)).max(0));
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONUP => {
                let x = ((l & 0xffff) as f32 / app.scale) as i32;
                let y = (((l >> 16) & 0xffff) as f32 / app.scale) as i32;
                if (18..=82).contains(&x) && (48..=112).contains(&y) {
                    app.toggle();
                } else if (225..=340).contains(&x) && y < 38 {
                    app.model_menu();
                } else if (344..=399).contains(&x) && y < 38 {
                    app.toggle_topmost();
                } else if (403..=453).contains(&x) && y < 38 {
                    app.open_settings();
                } else if x >= 461 && y < 38 {
                    app.stop(false);
                    ShowWindow(hwnd, SW_HIDE);
                } else if (289..=351).contains(&x)
                    && (app.layout_height - 75..=app.layout_height - 42).contains(&y)
                {
                    app.clear_text();
                } else if (359..=426).contains(&x)
                    && (app.layout_height - 75..=app.layout_height - 42).contains(&y)
                {
                    app.copy();
                } else if x >= 435 && (app.layout_height - 75..=app.layout_height - 42).contains(&y)
                {
                    app.retry_input();
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_EXITSIZEMOVE => {
                let mut r: RECT = zeroed();
                GetWindowRect(hwnd, &mut r);
                app.settings.position = Some((r.left, r.top));
                let _ = app.settings.save(&app.home);
                0
            }
            TRAY_MSG => {
                match l as u32 {
                    WM_LBUTTONUP => {
                        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => app.tray_menu(),
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                app.stop(false);
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_DESTROY => {
                let _ = app.worker.tx.send(Command::Quit);
                UnregisterHotKey(hwnd, HOTKEY);
                UnregisterHotKey(hwnd, ESCAPE_HOTKEY);
                let mut nid: NOTIFYICONDATAW = zeroed();
                nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !ptr.is_null() {
            let app = &mut *ptr;
            match msg {
                WM_COMMAND if w & 0xffff == 104 => {
                    app.save_settings();
                    return 0;
                }
                WM_CLOSE => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                WM_DESTROY => {
                    app.settings_hwnd = null_mut();
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
pub fn run(home: PathBuf, probe: bool) -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let focus_hook = input::focus_hook();
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let scale = GetDpiForSystem() as f32 / 96.;
        let scaled_width = (WIDTH as f32 * scale).round() as i32;
        let scaled_height = (HEIGHT as f32 * scale).round() as i32;
        let mut fonts = [null_mut(); 4];
        for (i, (height, weight)) in [
            (16, 400),
            (17, 600),
            (13, 400),
            ((16.0 * scale).round() as i32, 400),
        ]
        .iter()
        .enumerate()
        {
            fonts[i] = CreateFontW(
                -height,
                0,
                0,
                0,
                *weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Microsoft YaHei UI").as_ptr(),
            );
        }
        let icon = make_icon(instance);
        for (name, proc) in [
            (
                "FastAsrOverlay",
                Some(wndproc as unsafe extern "system" fn(_, _, _, _) -> _),
            ),
            (
                "FastAsrSettings",
                Some(settings_proc as unsafe extern "system" fn(_, _, _, _) -> _),
            ),
        ] {
            let mut wc: WNDCLASSEXW = zeroed();
            wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
            wc.lpfnWndProc = proc;
            wc.hInstance = instance;
            wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
            wc.hIcon = icon;
            wc.hIconSm = icon;
            wc.hbrBackground = (if name == "FastAsrSettings" {
                COLOR_BTNFACE + 1
            } else {
                COLOR_WINDOW + 1
            }) as HBRUSH;
            let name = wide(name);
            wc.lpszClassName = name.as_ptr();
            if RegisterClassExW(&wc) == 0 {
                return Err("无法注册窗口".into());
            }
        }
        let settings = Settings::load(&home);
        let position = settings.position.unwrap_or((
            (GetSystemMetrics(SM_CXSCREEN) - scaled_width) / 2,
            GetSystemMetrics(SM_CYSCREEN) - scaled_height - 90,
        ));
        let work = Worker::spawn(home.clone(), settings.model);
        let mut app = Box::new(App {
            hwnd: null_mut(),
            settings_hwnd: null_mut(),
            home,
            settings,
            worker: work,
            ready: false,
            listening: false,
            start_pending: false,
            stop_pending: false,
            hold_pressed: false,
            busy: true,
            target: None,
            paused: false,
            focus_paused: false,
            discard_cancelled_text: false,
            layout_height: HEIGHT,
            content_height: 54,
            scroll: 0,
            last_preview: String::new(),
            pending: VecDeque::new(),
            transcript: String::new(),
            partial: String::new(),
            notice: "正在加载本地识别模型…".into(),
            level: 0.,
            scale,
            fonts,
            icon,
            devices: Vec::new(),
            model_combo: null_mut(),
            mic_combo: null_mut(),
            pause_combo: null_mut(),
            hotkey_mode_combo: null_mut(),
            vad_combo: null_mut(),
            min_speech_combo: null_mut(),
            hotkey_registered: false,
            alt_hotkey: None,
            probe_stage: if probe { Some(0) } else { None },
            probe_benchmark: None,
        });
        let hwnd = CreateWindowExW(
            (if app.settings.topmost {
                WS_EX_TOPMOST
            } else {
                0
            }) | WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW,
            wide("FastAsrOverlay").as_ptr(),
            wide("Fast ASR").as_ptr(),
            WS_POPUP,
            position.0.max(0).min(GetSystemMetrics(SM_CXSCREEN) - 80),
            position.1.max(0).min(GetSystemMetrics(SM_CYSCREEN) - 50),
            scaled_width,
            scaled_height,
            null_mut(),
            null_mut(),
            instance,
            &mut *app as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err("无法创建悬浮条".into());
        }
        app.hwnd = hwnd;
        SetWindowRgn(
            hwnd,
            CreateRoundRectRgn(
                0,
                0,
                scaled_width + 1,
                scaled_height + 1,
                (18.0 * scale) as i32,
                (18.0 * scale) as i32,
            ),
            1,
        );
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = TRAY_MSG;
        nid.hIcon = icon;
        let tip = wide("Fast ASR · 本地语音输入");
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        Shell_NotifyIconW(NIM_ADD, &nid);
        app.set_hotkey();
        SetTimer(hwnd, 1, 50, None);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if !probe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !app.settings_hwnd.is_null() && IsDialogMessageW(app.settings_hwnd, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !focus_hook.is_null() {
            UnhookWinEvent(focus_hook);
        }
        for font in app.fonts {
            DeleteObject(font);
        }
        DestroyIcon(app.icon);
        Ok(())
    }
}
unsafe fn make_icon(instance: HINSTANCE) -> HICON {
    unsafe {
        let mask = [0u8; 128];
        let mut pixels = vec![0u8; 32 * 32 * 4];
        for y in 0i32..32 {
            for x in 0i32..32 {
                let i = ((y * 32 + x) * 4) as usize;
                let round = (x - 16).pow(2) + (y - 16).pow(2) < 225;
                let mic = ((13..=18).contains(&x) && (7..=19).contains(&y))
                    || ((10..=21).contains(&x) && (19..=21).contains(&y))
                    || ((15..=16).contains(&x) && (21..=26).contains(&y))
                    || ((12..=19).contains(&x) && y == 26);
                let (r, g, b) = if mic {
                    (17, 23, 32)
                } else if round {
                    (45, 217, 180)
                } else {
                    (17, 23, 32)
                };
                pixels[i..i + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
        CreateIcon(instance, 32, 32, 1, 32, mask.as_ptr(), pixels.as_ptr())
    }
}

unsafe fn save_window(hwnd: HWND, path: &std::path::Path) -> Result<(), String> {
    unsafe {
        let dc = GetDC(hwnd);
        let mut rect: RECT = zeroed();
        GetClientRect(hwnd, &mut rect);
        let memory = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, rect.right, rect.bottom);
        let old = SelectObject(memory, bitmap);
        SendMessageW(
            hwnd,
            WM_PRINT,
            memory as usize,
            (PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
        );
        let result = write_bitmap(memory, bitmap, rect.right, rect.bottom, path);
        SelectObject(memory, old);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(hwnd, dc);
        result
    }
}
unsafe fn write_bitmap(
    dc: HDC,
    bitmap: HBITMAP,
    width: i32,
    height: i32,
    path: &std::path::Path,
) -> Result<(), String> {
    unsafe {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        if GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        ) == 0
        {
            return Err("Cannot capture application preview".into());
        }
        let mut file = Vec::new();
        file.extend(b"BM");
        file.extend((54 + pixels.len() as u32).to_le_bytes());
        file.extend([0u8; 4]);
        file.extend(54u32.to_le_bytes());
        file.extend(40u32.to_le_bytes());
        file.extend(width.to_le_bytes());
        file.extend(height.to_le_bytes());
        file.extend(1u16.to_le_bytes());
        file.extend(32u16.to_le_bytes());
        file.extend(0u32.to_le_bytes());
        file.extend((pixels.len() as u32).to_le_bytes());
        file.extend([0u8; 16]);
        file.extend(pixels);
        std::fs::write(path, file).map_err(|e| e.to_string())
    }
}
