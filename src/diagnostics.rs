use crate::{
    assets::{self},
    audio,
    engine::{Engine, TextEvent},
    input::{self, Target, wide},
};
use serde_json::{Value, json};
use std::{
    fs,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    time::Instant,
};
use windows_sys::Win32::{
    System::{ProcessStatus::*, Threading::GetCurrentProcess},
    UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
};

pub fn executable_sha256() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn memory() -> Value {
    unsafe {
        let mut p: PROCESS_MEMORY_COUNTERS = zeroed();
        p.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb);
        json!({"working_set_mib":p.WorkingSetSize as f64/1048576.,"peak_working_set_mib":p.PeakWorkingSetSize as f64/1048576.,"private_commit_mib":p.PagefileUsage as f64/1048576.})
    }
}
fn wav(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Expected PCM WAV".into());
    }
    let u16at = |p: usize| u16::from_le_bytes([bytes[p], bytes[p + 1]]);
    let u32at = |p: usize| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    let mut pos = 12;
    let mut format_ok = false;
    let mut data = None;
    while pos + 8 <= bytes.len() {
        let len = u32at(pos + 4) as usize;
        let start = pos + 8;
        let end = start.checked_add(len).ok_or("WAV size overflow")?;
        if end > bytes.len() {
            return Err("Truncated WAV".into());
        }
        if &bytes[pos..pos + 4] == b"fmt " && len >= 16 {
            format_ok = u16at(start) == 1
                && u16at(start + 2) == 1
                && u32at(start + 4) == 16000
                && u16at(start + 14) == 16;
        }
        if &bytes[pos..pos + 4] == b"data" {
            data = Some((start, end));
        }
        pos = end + (len % 2);
    }
    if !format_ok {
        return Err("Expected 16 kHz mono PCM16 WAV".into());
    }
    let (a, b) = data.ok_or("WAV has no data chunk")?;
    Ok(bytes[a..b]
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.)
        .collect())
}
fn recognize(engine: &mut Engine, audio: &[f32]) -> Result<Value, String> {
    engine.reset(600)?;
    let start = Instant::now();
    let mut texts = Vec::new();
    let mut partial_updates = 0;
    for chunk in audio.chunks(512) {
        let mut frame = [0.; 512];
        frame[..chunk.len()].copy_from_slice(chunk);
        for event in engine.feed(&frame)? {
            match event {
                TextEvent::Final(s) => texts.push(s),
                TextEvent::Partial(s) => {
                    if !s.is_empty() {
                        partial_updates += 1;
                    }
                }
            }
        }
    }
    for _ in 0..40 {
        for event in engine.feed(&[0.; 512])? {
            match event {
                TextEvent::Final(s) => texts.push(s),
                TextEvent::Partial(s) => {
                    if !s.is_empty() {
                        partial_updates += 1;
                    }
                }
            }
        }
    }
    if let Some(TextEvent::Final(s)) = engine.finish()? {
        texts.push(s);
    }
    let seconds = audio.len() as f64 / 16000.;
    let elapsed = start.elapsed().as_secs_f64();
    Ok(
        json!({"audio_seconds":seconds,"processing_seconds":elapsed,"real_time_factor":elapsed/seconds.max(0.01),"segments":texts,"partial_updates":partial_updates,"memory":memory()}),
    )
}
pub fn run(home: &Path, args: &[String]) -> Result<PathBuf, String> {
    let report_arg = args
        .iter()
        .position(|a| a == "--report")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let report = report_arg.unwrap_or_else(|| home.join("reports/self-test.json"));
    fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let kind = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .map(|s| assets::ModelKind::from_name(s))
        .unwrap_or(assets::ModelKind::XAsr480);
    let paths = assets::ensure(home, kind, |_| {})?;
    let mut engine = Engine::new(&paths)?;
    let loaded = memory();
    let load_time = start.elapsed().as_secs_f64();
    let mut cases = Vec::new();
    let sample_dir = paths.samples.clone();
    if let Some(i) = args.iter().position(|a| a == "--wav") {
        let path = PathBuf::from(args.get(i + 1).ok_or("Missing --wav path")?);
        cases.push((path.clone(), wav(&path)?));
    } else if sample_dir.is_dir() {
        let mut files: Vec<_> = fs::read_dir(&sample_dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|f| f.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "wav")
                    && p.file_name().is_some_and(|n| n != "8k.wav")
            })
            .collect();
        files.sort();
        for p in files {
            cases.push((p.clone(), wav(&p)?));
        }
    }
    let mut results = Vec::new();
    for (path, samples) in &cases {
        let mut result = recognize(&mut engine, samples)?;
        result["file"] = json!(path.file_name().unwrap().to_string_lossy());
        results.push(result);
    }
    let silence = recognize(&mut engine, &vec![0.; 16000 * 10])?;
    let loops = args
        .iter()
        .position(|a| a == "--repeat")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 1000);
    let stress_start = Instant::now();
    let mut stress_audio = 0.;
    let mut stress_results = Vec::new();
    for index in 1..loops {
        for (_, samples) in &cases {
            let _ = recognize(&mut engine, samples)?;
            stress_audio += samples.len() as f64 / 16000.;
        }
        if index % 5 == 0 || index == loops - 1 {
            stress_results.push(json!({"iteration":index,"memory":memory()}));
        }
    }
    let mic_list = audio::devices();
    let mut mic_reports = Vec::new();
    if args.iter().any(|a| a == "--capture-probe") {
        for (id, name) in &mic_list {
            let mut result = json!({"id":id,"name":name});
            match audio::Recorder::open(*id) {
                Ok(mut rec) => {
                    let begin = Instant::now();
                    let mut peak = 0f32;
                    while begin.elapsed().as_secs_f32() < 0.65 {
                        for chunk in rec.drain()? {
                            for s in chunk {
                                peak = peak.max(s.abs());
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(8));
                    }
                    result["samples"] = json!(rec.sample_count);
                    result["peak"] = json!(peak);
                    result["ok"] = json!(rec.sample_count > 0);
                }
                Err(e) => {
                    result["error"] = json!(e);
                    result["ok"] = json!(false);
                }
            }
            mic_reports.push(result);
        }
    }
    let final_memory = memory();
    let recognized = !results.is_empty()
        && results
            .iter()
            .all(|r| r["segments"].as_array().is_some_and(|s| !s.is_empty()));
    let silence_empty = silence["segments"].as_array().is_some_and(|s| s.is_empty());
    let content = json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,"model":paths.model.file_name().unwrap().to_string_lossy(),"weights_precision":paths.kind.name(),"runtime":"sherpa-onnx 1.13.8 CPU / native streaming endpoint","model_load_seconds":load_time,"loaded_memory":loaded,"recognition_cases":results,"silence_case":silence,"stress":{"repeat":loops,"additional_audio_seconds":stress_audio,"elapsed_seconds":stress_start.elapsed().as_secs_f64(),"checkpoints":stress_results},"capture":mic_reports,"final_memory":final_memory,"passed":recognized&&silence_empty,"limits":{"memory_metric":"process working set, measured by GetProcessMemoryInfo; this diagnostic has no GUI"}});
    fs::write(&report, serde_json::to_vec_pretty(&content).unwrap()).map_err(|e| e.to_string())?;
    Ok(report)
}

/// Focus only a diagnostic-owned window on an interactive desktop.
/// Production dictation never uses this foreground activation helper.
pub unsafe fn focus_probe_window(
    host: windows_sys::Win32::Foundation::HWND,
    edit: windows_sys::Win32::Foundation::HWND,
) {
    unsafe {
        // Never attach another application's input queue: a hung foreground
        // window can block AttachThreadInput / activation indefinitely. The
        // caller verifies foreground and focus before sending any test input.
        SetForegroundWindow(host);
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(host);
        SetFocus(edit);
    }
}

pub fn input_probe(home: &Path) -> Result<PathBuf, String> {
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        let instance = GetModuleHandleW(std::ptr::null());
        let mut cls: WNDCLASSW = zeroed();
        cls.lpfnWndProc = Some(DefWindowProcW);
        cls.hInstance = instance;
        let name = wide("FastAsrInputProbe");
        cls.lpszClassName = name.as_ptr();
        RegisterClassW(&cls);
        let host = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("Fast ASR · 输入自测（自动关闭）").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            600,
            200,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | 0x0004,
            20,
            20,
            530,
            50,
            host,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        let second = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            20,
            90,
            530,
            30,
            host,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        ShowWindow(host, SW_SHOWNORMAL);
        ShowWindow(host, SW_SHOW);
        focus_probe_window(host, edit);
        pump(150);
        let target =
            Target::capture(std::ptr::null_mut()).ok_or("Cannot capture test input focus")?;
        if target.window != host as isize || target.focus != edit as isize {
            DestroyWindow(host);
            return Err("Test window did not acquire focus; no text was sent".into());
        }
        let target_title = target.title();
        let expected = "中文 English API 123 😀";
        let send = input::type_text(target, expected);
        pump(200);
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(edit, buf.as_mut_ptr(), 512);
        let actual = String::from_utf16_lossy(&buf[..n as usize]);
        SetFocus(second);
        pump(50);
        let focus_guard = input::type_text(target, "SHOULD_NOT_APPEAR").is_err();
        let mut check = [0u16; 64];
        let n2 = GetWindowTextW(second, check.as_mut_ptr(), 64);
        DestroyWindow(host);
        let report = home.join("reports/input-probe.json");
        fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&report,serde_json::to_vec_pretty(&json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,"passed":send.is_ok()&&actual==expected&&focus_guard&&n2==0,"target_title":target_title,"unicode_input":{"expected":expected,"actual":actual,"error":send.err()},"changed_focus_blocked":focus_guard,"second_input_untouched":n2==0})).unwrap()).map_err(|e|e.to_string())?;
        Ok(report)
    }
}
unsafe fn pump(ms: u64) {
    unsafe {
        let start = Instant::now();
        let mut msg: MSG = zeroed();
        while start.elapsed().as_millis() < (ms as u128) {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

pub fn benchmark_decoder(
    decoder: &mut crate::engine::Decoder,
    paths: &assets::Paths,
) -> Result<Value, String> {
    let path = paths.samples.join("46.wav");
    let audio = wav(&path)?;
    let start = Instant::now();
    let text = decoder.transcribe(&audio)?;
    Ok(
        json!({"audio_seconds":audio.len() as f64/16000.,"elapsed_seconds":start.elapsed().as_secs_f64(),"text":text,"memory":memory(),"passed":!text.is_empty()}),
    )
}

pub fn pipeline_probe(home: &Path) -> Result<PathBuf, String> {
    use crate::{
        assets::ModelKind,
        config::Settings,
        worker::{Command, Event, Worker},
    };
    use std::time::Duration;
    fn send(worker: &Worker, command: Command) -> Result<(), String> {
        worker.tx.send(command).map_err(|e| e.to_string())
    }
    fn ready(worker: &Worker) -> Result<Value, String> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            match worker.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::Ready) => {
                    return Ok(
                        json!({"load_seconds":start.elapsed().as_secs_f64(),"memory":memory()}),
                    );
                }
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Model switch timed out".into())
    }
    fn bench(worker: &Worker) -> Result<Value, String> {
        send(worker, Command::Benchmark)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            match worker.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::Benchmarked(v)) => return Ok(v),
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Benchmark timed out".into())
    }
    fn replay(worker: &Worker, samples: &[f32]) -> Result<Value, String> {
        send(
            worker,
            Command::Replay(Settings::default(), samples.to_vec()),
        )?;
        let start = Instant::now();
        let mut partials = 0;
        let mut finals = Vec::new();
        let mut captured = 0;
        let mut first_partial = None;
        let mut first_final = None;
        let mut capture_before_first_final = 0;
        while start.elapsed() < Duration::from_secs(120) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Partial(s)) if !s.is_empty() => {
                    partials += 1;
                    if first_partial.is_none() {
                        first_partial = Some(start.elapsed().as_secs_f64());
                    }
                }
                Ok(Event::Final(s)) => {
                    if first_final.is_none() {
                        first_final = Some(start.elapsed().as_secs_f64());
                        capture_before_first_final = captured;
                    }
                    finals.push(s);
                }
                Ok(Event::Captured(n)) => captured = n,
                Ok(Event::Stopped) => {
                    return Ok(
                        json!({"passed":captured==samples.len() as u64&&!finals.is_empty()&&partials>0&&first_partial<first_final,
                "captured_samples":captured,"expected_samples":samples.len(),"captured_before_first_final":capture_before_first_final,
                "partial_updates":partials,"first_partial_seconds":first_partial,"first_final_seconds":first_final,
                "finals":finals,"elapsed_seconds":start.elapsed().as_secs_f64(),"memory":memory()}),
                    );
                }
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Replay did not finish".into())
    }
    fn held_replay(
        worker: &Worker,
        samples: &[f32],
        cancel: bool,
        streaming: bool,
    ) -> Result<Value, String> {
        let mut padded = samples.to_vec();
        padded.resize(samples.len() + 16000 * 8, 0.0);
        send(worker, Command::ReplayHeld(Settings::default(), padded))?;
        let start = Instant::now();
        let mut before_release = 0;
        let mut partials_before_release = 0;
        let mut finals_after_release = 0;
        let mut finals = Vec::new();
        let mut captured = 0;
        let mut released = false;
        while start.elapsed() < Duration::from_secs(120) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Captured(n)) => {
                    captured = n;
                    if !released && n >= (samples.len() + 16000) as u64 {
                        send(
                            worker,
                            if cancel {
                                Command::Cancel
                            } else {
                                Command::Stop
                            },
                        )?;
                        released = true;
                    }
                }
                Ok(Event::Partial(s)) if !s.is_empty() => {
                    if !released {
                        before_release += 1;
                        partials_before_release += 1;
                    }
                }
                Ok(Event::Final(s)) => {
                    if !released {
                        before_release += 1;
                    }
                    if released {
                        finals_after_release += 1;
                    }
                    finals.push(s);
                }
                Ok(Event::Stopped) => {
                    return Ok(json!({
                        "passed": released && (!streaming || partials_before_release > 0)
                            && if cancel { finals_after_release == 0 } else { !finals.is_empty() },
                        "streaming":streaming,"partials_before_release":partials_before_release,
                        "finals_after_release":finals_after_release,
                        "text_events_before_release": before_release, "explicit_release": released,
                        "cancelled": cancel, "captured_samples": captured, "finals": finals,
                        "elapsed_seconds": start.elapsed().as_secs_f64()
                    }));
                }
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Held replay timed out".into())
    }
    fn immediate_release(worker: &Worker, samples: &[f32]) -> Result<bool, String> {
        send(
            worker,
            Command::ReplayHeld(Settings::default(), samples.to_vec()),
        )?;
        send(worker, Command::Stop)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Stopped) => return Ok(true),
                Ok(Event::Final(s)) if !s.is_empty() => return Ok(false),
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Immediate release timed out".into())
    }
    fn silent_replay(worker: &Worker) -> Result<bool, String> {
        send(
            worker,
            Command::Replay(Settings::default(), vec![0.; 16000 * 3]),
        )?;
        let start = Instant::now();
        let mut captured = 0;
        while start.elapsed() < Duration::from_secs(20) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Captured(n)) => captured = n,
                Ok(Event::Stopped) => return Ok(captured == 16000 * 3),
                Ok(Event::Partial(s) | Event::Final(s)) if !s.is_empty() => return Ok(false),
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Silent replay timed out".into())
    }
    fn cancel(worker: &Worker, samples: &[f32]) -> Result<bool, String> {
        send(
            worker,
            Command::Replay(Settings::default(), samples.to_vec()),
        )?;
        let start = Instant::now();
        let mut requested = false;
        while start.elapsed() < Duration::from_secs(30) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Partial(s)) if !s.is_empty() && !requested => {
                    send(worker, Command::Cancel)?;
                    requested = true;
                }
                Ok(Event::Stopped) => return Ok(requested),
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Cancel timed out".into())
    }
    let paths = assets::Paths::resolve(home, ModelKind::XAsr480);
    let samples = wav(&paths.samples.join("46.wav"))?;
    let worker = Worker::spawn(home.to_path_buf(), ModelKind::XAsr480);
    let result = (|| -> Result<Value, String> {
        let initial = ready(&worker)?;
        let first = replay(&worker, &samples)?;
        let held = held_replay(&worker, &samples, false, true)?;
        let held_cancel = held_replay(&worker, &samples, true, true)?;
        let quick_release = immediate_release(&worker, &samples)?;
        let silence = silent_replay(&worker)?;
        let cancelled = cancel(&worker, &samples)?;
        let restart = replay(&worker, &samples)?;
        let benchmark = bench(&worker)?;
        let mut separated = samples.clone();
        separated.resize(samples.len() + 16000 * 2, 0.);
        separated.extend_from_slice(&samples);
        separated.resize(separated.len() + 16000 * 2, 0.);
        let continuous = replay(&worker, &separated)?;
        let endpoints = continuous["finals"]
            .as_array()
            .is_some_and(|v| v.len() >= 2)
            && continuous["first_final_seconds"]
                .as_f64()
                .is_some_and(|t| t < samples.len() as f64 / 16000. + 2.);
        send(&worker, Command::Retry)?;
        let reload = ready(&worker)?;
        let last = replay(&worker, &samples)?;
        Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,
            "source":"WAV replay at real-time pace; microphone closed; no external VAD",
            "passed":held["passed"]==true&&held_cancel["passed"]==true&&quick_release&&silence&&first["passed"]==true&&cancelled&&restart["passed"]==true&&benchmark["passed"]==true&&last["passed"]==true&&continuous["passed"]==true&&endpoints&&first["finals"]==restart["finals"]&&first["finals"]==last["finals"],
            "held_release":held,"held_cancel":held_cancel,"immediate_release":quick_release,"silence_has_no_text":silence,
            "initial_load":initial,"replay":first,"cancel_after_partial":cancelled,"restart":restart,
            "benchmark":benchmark,"continuous_two_utterances":continuous,"native_endpoint_splits":endpoints,"reload":reload,"after_reload_replay":last}),
        )
    })();
    let _ = worker.tx.send(Command::Quit);
    let report = home.join("reports/pipeline-probe.json");
    fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&report, serde_json::to_vec_pretty(&result?).unwrap()).map_err(|e| e.to_string())?;
    Ok(report)
}

/// Exercise the installed left Alt hook against our own EDIT; never opens a microphone.
pub fn hotkey_probe(home: &Path) -> Result<PathBuf, String> {
    use crate::hotkey::{self, AltHotkey};
    use std::{thread, time::Duration};
    use windows_sys::Win32::{
        Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::Input::KeyboardAndMouse::*,
    };
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            // Do not enter a modal system-menu loop in the diagnostic host.
            if msg == WM_SYSCOMMAND && w & 0xfff0 == SC_KEYMENU as usize {
                return 0;
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
    }
    #[derive(Default)]
    struct Counts {
        starts: u32,
        releases: u32,
        cancels: u32,
        failures: u32,
        native_alt: u32,
        native_alt_up: u32,
        native_combo: u32,
        native_mouse: u32,
        native_ctrl_combo: u32,
    }
    fn pump(hook: &AltHotkey, counts: &mut Counts, millis: u64) {
        let _ = fs::write(
            crate::config::home().join("reports/hotkey-stage.txt"),
            format!(
                "pump {millis} start={},{},{} trace={:?}",
                counts.starts,
                counts.releases,
                counts.cancels,
                hook.diagnostic_trace()
            ),
        );
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(millis) {
            unsafe {
                let mut msg = zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    match msg.message {
                        hotkey::HOLD_RELEASE => counts.releases += 1,
                        hotkey::HOLD_CANCEL => counts.cancels += 1,
                        hotkey::HOLD_FAILED => counts.failures += 1,
                        WM_SYSKEYDOWN | WM_KEYDOWN
                            if [VK_MENU as usize, VK_LMENU as usize, VK_RMENU as usize]
                                .contains(&msg.wParam) =>
                        {
                            counts.native_alt += 1
                        }
                        WM_SYSKEYDOWN | WM_KEYDOWN
                            if msg.wParam == 0x41 && GetKeyState(VK_MENU as i32) < 0 =>
                        {
                            counts.native_combo += 1
                        }
                        WM_KEYDOWN if msg.wParam == 0x41 && GetKeyState(VK_CONTROL as i32) < 0 => {
                            counts.native_ctrl_combo += 1
                        }
                        WM_MOUSEWHEEL if GetKeyState(VK_MENU as i32) < 0 => {
                            counts.native_mouse += 1
                        }
                        _ => {}
                    }
                    // Native Alt-up and access keys can enter a modal menu loop in
                    // the stock EDIT. Observe the delivered events without opening
                    // that loop; the diagnostic never operates an application's menu.
                    if msg.message == WM_SYSKEYUP && msg.wParam == VK_MENU as usize {
                        counts.native_alt_up += 1;
                        continue;
                    }
                    if matches!(msg.message, WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR) {
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if hook.poll(true) {
                counts.starts += 1;
            }
            thread::sleep(Duration::from_millis(3));
        }
    }
    fn key(host: HWND, edit: HWND, vk: u16, up: bool) -> Result<(), String> {
        unsafe {
            // A native short Alt tap may move focus to this window's menu.
            // Restore only our own edit; never send test keys to another window.
            if !up && GetForegroundWindow() == host && GetFocus() != edit {
                SetFocus(edit);
            }
            if !up && (GetForegroundWindow() != host || GetFocus() != edit) {
                return Err(format!(
                    "Hotkey test lost focus before key {vk}: host={}, foreground={}, edit={}, focus={}; no key was sent",
                    host as usize,
                    GetForegroundWindow() as usize,
                    edit as usize,
                    GetFocus() as usize
                ));
            }
            let event = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }
                            | if vk == VK_RMENU {
                                KEYEVENTF_EXTENDEDKEY
                            } else {
                                0
                            },
                        time: 0,
                        dwExtraInfo: hotkey::TEST_INPUT,
                    },
                },
            };
            if SendInput(1, &event, size_of::<INPUT>() as i32) != 1 {
                return Err("Hotkey test SendInput failed".into());
            }
            Ok(())
        }
    }
    fn wheel(host: HWND, edit: HWND) -> Result<std::thread::JoinHandle<bool>, String> {
        unsafe {
            if GetForegroundWindow() != host || GetFocus() != edit {
                return Err("Mouse test lost its owned input focus; no input sent".into());
            }
            let mut rect = zeroed();
            GetWindowRect(edit, &mut rect);
            let mut cursor = zeroed();
            GetCursorPos(&mut cursor);
            if cursor.x < rect.left
                || cursor.x >= rect.right
                || cursor.y < rect.top
                || cursor.y >= rect.bottom
            {
                return Err("Mouse test cursor left the owned input; no input sent".into());
            }
            // Inject mouse input from a separate source thread while the owner
            // pumps its installed hooks, just as with an external input device.
            let host_id = host as usize;
            let edit_id = edit as usize;
            Ok(thread::spawn(move || {
                let mut point = zeroed();
                GetCursorPos(&mut point);
                if GetForegroundWindow() as usize != host_id
                    || WindowFromPoint(point) as usize != edit_id
                {
                    return false;
                }
                let event = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: 120,
                            dwFlags: MOUSEEVENTF_WHEEL,
                            time: 0,
                            dwExtraInfo: hotkey::TEST_INPUT,
                        },
                    },
                };
                SendInput(1, &event, size_of::<INPUT>() as i32) == 1
            }))
        }
    }
    unsafe {
        let _ = fs::write(home.join("reports/hotkey-stage.txt"), "create");
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("FastAsrAltProbe");
        let mut wc: WNDCLASSEXW = zeroed();
        wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(proc);
        wc.hInstance = instance;
        wc.lpszClassName = name.as_ptr();
        if RegisterClassExW(&wc) == 0 {
            return Err("Cannot register hotkey diagnostic window".into());
        }
        let host = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("Fast ASR · 左 Alt 快捷键自测（不录音）").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            600,
            200,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            20,
            20,
            530,
            50,
            host,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        ShowWindow(host, SW_SHOW);
        let _ = fs::write(home.join("reports/hotkey-stage.txt"), "focus");
        focus_probe_window(host, edit);
        let _ = fs::write(home.join("reports/hotkey-stage.txt"), "activate-drain");
        // Drain activation before installing the hook or sending synthetic keys.
        for _ in 0..30 {
            let mut msg = zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(5));
        }
        let mut original_cursor = zeroed();
        GetCursorPos(&mut original_cursor);
        let hook = AltHotkey::install(host, true)?;
        let outcome = (|| -> Result<Value, String> {
            if GetForegroundWindow() != host || GetFocus() != edit {
                return Err("Hotkey test could not focus its own input".into());
            }
            let mut counts = Counts::default();
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 160);
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 120);
            let short_alt = counts.starts == 0
                && counts.releases == 0
                && counts.native_alt > 0
                && counts.native_alt_up > 0;
            let native_before = counts.native_alt;
            let alt_up_before = counts.native_alt_up;
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 440);
            let no_early_start = counts.starts == 0;
            pump(&hook, &mut counts, 180);
            let start_after_threshold = counts.starts == 1;
            for _ in 0..3 {
                key(host, edit, VK_LMENU, false)?;
                pump(&hook, &mut counts, 30);
            }
            let once_per_hold = counts.starts == 1;
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 120);
            let release = counts.releases == 1;
            let no_menu =
                counts.native_alt == native_before && counts.native_alt_up == alt_up_before;
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 100);
            key(host, edit, 0x41, false)?;
            pump(&hook, &mut counts, 60);
            key(host, edit, 0x41, true)?;
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 120);
            let combo = counts.native_combo > 0 && counts.starts == 1;
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 610);
            key(host, edit, VK_ESCAPE, false)?;
            pump(&hook, &mut counts, 60);
            key(host, edit, VK_ESCAPE, true)?;
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 100);
            let cancel = counts.starts == 2 && counts.cancels == 1 && counts.releases == 1;
            // Ctrl shortcuts now bypass the recording gesture completely.
            let starts_before = counts.starts;
            key(host, edit, VK_LCONTROL, false)?;
            pump(&hook, &mut counts, 600);
            key(host, edit, 0x41, false)?;
            pump(&hook, &mut counts, 60);
            key(host, edit, 0x41, true)?;
            key(host, edit, VK_LCONTROL, true)?;
            pump(&hook, &mut counts, 100);
            let ctrl_untouched = counts.starts == starts_before && counts.native_ctrl_combo > 0;
            key(host, edit, VK_RMENU, false)?;
            pump(&hook, &mut counts, 600);
            key(host, edit, VK_RMENU, true)?;
            pump(&hook, &mut counts, 100);
            let right_alt_untouched = counts.starts == starts_before;
            // Keep all pointer input inside the diagnostic-owned EDIT.
            let mut edit_rect = zeroed();
            GetWindowRect(edit, &mut edit_rect);
            SetCursorPos(edit_rect.left + 40, edit_rect.top + 20);
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 100);
            let wheel_input = wheel(host, edit)?;
            pump(&hook, &mut counts, 100);
            if !wheel_input.is_finished() || !wheel_input.join().unwrap_or(false) {
                return Err(
                    "Mouse injection did not complete while pumping the owned input".into(),
                );
            }
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 100);
            let mouse_pending = counts.starts == starts_before && counts.native_mouse > 0;
            let mouse_before = counts.native_mouse;
            let cancels_before = counts.cancels;
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 610);
            let wheel_input = wheel(host, edit)?;
            pump(&hook, &mut counts, 100);
            if !wheel_input.is_finished() || !wheel_input.join().unwrap_or(false) {
                return Err(
                    "Mouse injection did not complete while pumping the owned input".into(),
                );
            }
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 100);
            let mouse_recording = counts.starts == starts_before + 1
                && counts.cancels == cancels_before + 1
                && counts.native_mouse > mouse_before;
            // A keyboard chord after the threshold must cancel, not submit.
            let cancels_before = counts.cancels;
            let combos_before = counts.native_combo;
            key(host, edit, VK_LMENU, false)?;
            pump(&hook, &mut counts, 610);
            key(host, edit, 0x41, false)?;
            pump(&hook, &mut counts, 70);
            key(host, edit, 0x41, true)?;
            key(host, edit, VK_LMENU, true)?;
            pump(&hook, &mut counts, 100);
            let chord_cancels = counts.cancels == cancels_before + 1
                && counts.native_combo > combos_before
                && counts.releases == 1;
            let no_stuck_modifiers = !input::modifiers_down();
            let no_characters = GetWindowTextLengthW(edit) == 0;
            Ok(
                json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,
                "source":"Installed Windows low-level left Alt hook, synthetic tagged keys, owned EDIT; no microphone",
                "passed":short_alt&&no_early_start&&start_after_threshold&&once_per_hold&&release&&no_menu&&combo&&cancel&&no_stuck_modifiers&&no_characters&&counts.failures==0&&ctrl_untouched&&right_alt_untouched&&mouse_pending&&mouse_recording&&chord_cancels,
                "short_alt_restored":short_alt,"no_start_before_500ms":no_early_start,"starts_after_500ms":start_after_threshold,
                "once_per_hold":once_per_hold,"release_submits":release,"long_alt_not_forwarded_to_native_menu":no_menu,
                "alt_letter_preserves_native_combo":combo,"escape_cancels":cancel,"ctrl_shortcuts_untouched":ctrl_untouched,"right_alt_no_recording":right_alt_untouched,"alt_mouse_before_threshold_forwards":mouse_pending,"alt_mouse_during_recording_cancels_and_forwards":mouse_recording,"keyboard_chord_during_recording_cancels_and_forwards":chord_cancels,"no_stuck_modifiers":no_stuck_modifiers,
                "no_typed_characters":no_characters,"replay_failures":counts.failures,"counts":{"start":counts.starts,"release":counts.releases,"cancel":counts.cancels,"alt":counts.native_alt,"combo":counts.native_combo,"mouse":counts.native_mouse,"ctrl_combo":counts.native_ctrl_combo},"trace":hook.diagnostic_trace()}),
            )
        })();
        if let Err(error) = &outcome {
            let trace = hook.diagnostic_trace();
            let _ = fs::write(
                home.join("reports/hotkey-error-trace.txt"),
                format!("{error}\n{trace:#?}"),
            );
        }
        // Always balance synthetic keys before closing the test window, including errors.
        let _ = key(host, edit, 0x41, true);
        let _ = key(host, edit, VK_ESCAPE, true);
        let _ = key(host, edit, VK_LMENU, true);
        let _ = key(host, edit, VK_RMENU, true);
        let _ = key(host, edit, VK_LCONTROL, true);
        drop(hook);
        SetCursorPos(original_cursor.x, original_cursor.y);
        DestroyWindow(host);
        let report = home.join("reports/hotkey-probe.json");
        fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&report, serde_json::to_vec_pretty(&outcome?).unwrap())
            .map_err(|e| e.to_string())?;
        Ok(report)
    }
}

/// Run only on a private desktop via scripts/check_hook_lifecycle.py.
/// No microphone, keyboard/mouse injection or foreground manipulation.
pub fn hook_lifecycle_probe(home: &Path) -> Result<PathBuf, String> {
    let mut cycles = 0;
    for _ in 0..5 {
        let mut hook = crate::hotkey::AltHotkey::install(std::ptr::null_mut(), false)?;
        if hook.poll(false) {
            return Err("Unexpected recording start".into());
        }
        hook.shutdown()?;
        cycles += 1;
    }
    let report = home.join("reports/hook-lifecycle-probe.json");
    fs::create_dir_all(home.join("reports")).map_err(|e| e.to_string())?;
    fs::write(
        &report,
        serde_json::to_vec_pretty(&json!({
            "version": env!("CARGO_PKG_VERSION"),
            "executable_sha256": executable_sha256()?,
            "process_id": std::process::id(), "cycles": cycles,
            "keyboard_and_mouse_hooks": true, "input_injected": false,
            "microphone_opened": false, "passed": cycles == 5,
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(report)
}
