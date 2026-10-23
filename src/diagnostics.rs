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
    let content = json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,"model":paths.model.file_name().unwrap().to_string_lossy(),"weights_precision":paths.kind.name(),"runtime":if paths.kind.streaming(){"sherpa-onnx 1.13.8 CPU / streaming endpoint"}else{"whisper.cpp 1.9.5 CPU / Silero VAD"},"model_load_seconds":load_time,"loaded_memory":loaded,"recognition_cases":results,"silence_case":silence,"stress":{"repeat":loops,"additional_audio_seconds":stress_audio,"elapsed_seconds":stress_start.elapsed().as_secs_f64(),"checkpoints":stress_results},"capture":mic_reports,"final_memory":final_memory,"passed":recognized&&silence_empty,"limits":{"memory_metric":"process working set, measured by GetProcessMemoryInfo; this diagnostic has no GUI"}});
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
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let attached = foreground != 0
            && foreground != current
            && AttachThreadInput(current, foreground, 1) != 0;
        SetForegroundWindow(host);
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(host);
        SetFocus(edit);
        if attached {
            AttachThreadInput(current, foreground, 0);
        }
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
        let cancelled = cancel(&worker, &samples)?;
        let restart = replay(&worker, &samples)?;
        send(&worker, Command::Switch(ModelKind::WhisperTurboQ5))?;
        let turbo_load = ready(&worker)?;
        let turbo = bench(&worker)?;
        send(&worker, Command::Switch(ModelKind::XAsr480))?;
        let reload = ready(&worker)?;
        let last = replay(&worker, &samples)?;
        Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,"source":"WAV replay at real-time pace; no microphone capture",
            "passed":first["passed"]==true&&cancelled&&restart["passed"]==true&&turbo["passed"]==true&&last["passed"]==true&&first["finals"]==restart["finals"]&&first["finals"]==last["finals"],
            "initial_load":initial,"replay":first,"cancel_after_partial":cancelled,"restart":restart,
            "turbo_load":turbo_load,"turbo_benchmark":turbo,"xasr_reload":reload,"after_switch_replay":last}),
        )
    })();
    let _ = worker.tx.send(Command::Quit);
    let report = home.join("reports/pipeline-probe.json");
    fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&report, serde_json::to_vec_pretty(&result?).unwrap()).map_err(|e| e.to_string())?;
    Ok(report)
}
