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
        let mut vad_speech = 0;
        let mut vad_quiet = 0;
        let mut vad_max = 0f32;
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
                Ok(Event::Vad {
                    probability,
                    speech,
                }) => {
                    vad_max = vad_max.max(probability);
                    if speech {
                        vad_speech += 1;
                    } else {
                        vad_quiet += 1;
                    }
                }
                Ok(Event::Captured(n)) => captured = n,
                Ok(Event::Stopped) => {
                    return Ok(
                        json!({"passed":captured==samples.len() as u64&&!finals.is_empty()&&partials>0&&first_partial<first_final&&vad_speech>0&&vad_quiet>0&&vad_max>=0.5,
                "captured_samples":captured,"expected_samples":samples.len(),"captured_before_first_final":capture_before_first_final,
                "vad":{"speech_events":vad_speech,"quiet_events":vad_quiet,"max_probability":vad_max},
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
    fn vad_parameters(paths: &assets::Paths, samples: &[f32]) -> Result<Value, String> {
        // The worker already owns the shared VAD library; retain that lifetime.
        let mut vad = crate::engine::Segmenter::new(paths)?;
        let mut padded = samples.to_vec();
        padded.resize(samples.len() + 16000 * 2, 0.);
        let mut run = |threshold: f32, minimum: u32, pause: u32| -> Result<Value, String> {
            let settings = Settings {
                vad_threshold: threshold,
                vad_min_speech_ms: minimum,
                silence_ms: pause,
                ..Settings::default()
            };
            vad.configure(&settings);
            let mut first = None;
            let mut ends = Vec::new();
            let mut approved = 0;
            let mut low_frames = 0;
            let mut high_frames = 0;
            let mut max_probability = 0f32;
            for (index, frame) in padded.chunks(512).enumerate() {
                let mut block = [0f32; 512];
                block[..frame.len()].copy_from_slice(frame);
                let (audio, ended) = vad.feed_stream(&block)?;
                max_probability = max_probability.max(vad.probability);
                low_frames += usize::from(vad.probability >= 0.2);
                high_frames += usize::from(vad.probability >= 0.8);
                if !audio.is_empty() && first.is_none() {
                    first = Some((index + 1) * 32);
                }
                approved += audio.len();
                if ended {
                    ends.push((index + 1) * 32);
                }
            }
            Ok(
                json!({"threshold":threshold,"minimum_speech_ms":minimum,"pause_ms":pause,
                "first_audio_ms":first,"segment_ends_ms":ends,"approved_samples":approved,
                "frames_over_0_2":low_frames,"frames_over_0_8":high_frames,
                "max_probability":max_probability}),
            )
        };
        let sensitive = run(0.2, 64, 400)?;
        let strict = run(0.8, 64, 400)?;
        let minimum_short = run(0.5, 64, 400)?;
        let minimum_long = run(0.5, 512, 400)?;
        let pause_long = run(0.5, 64, 1000)?;
        let first = |v: &Value| v["first_audio_ms"].as_u64().unwrap_or(u64::MAX);
        let last = |v: &Value| {
            v["segment_ends_ms"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let passed = first(&sensitive) < u64::MAX
            && first(&strict) >= first(&sensitive)
            && first(&minimum_long) > first(&minimum_short)
            && last(&pause_long) > last(&minimum_short)
            && sensitive["frames_over_0_2"].as_u64().unwrap_or(0)
                > sensitive["frames_over_0_8"].as_u64().unwrap_or(u64::MAX);
        Ok(
            json!({"passed":passed,"source":"Native Silero VAD, same WAV with 2s trailing silence, reset between settings",
            "sensitive":sensitive,"strict":strict,"minimum_short":minimum_short,
            "minimum_long":minimum_long,"pause_long":pause_long}),
        )
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
    fn silent_vad(worker: &Worker) -> Result<bool, String> {
        let settings = Settings {
            vad_threshold: 0.2,
            vad_min_speech_ms: 64,
            ..Settings::default()
        };
        send(worker, Command::Replay(settings, vec![0.0; 16000]))?;
        let start = Instant::now();
        let mut observed = false;
        while start.elapsed() < Duration::from_secs(20) {
            match worker.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Stopped) => return Ok(observed),
                Ok(Event::Vad {
                    probability,
                    speech,
                }) => {
                    if !probability.is_finite() || speech || probability >= 0.2 {
                        return Ok(false);
                    }
                    observed = true;
                }
                Ok(Event::Partial(s) | Event::Final(s)) if !s.is_empty() => return Ok(false),
                Ok(Event::Error(e) | Event::LoadFailed(e)) => return Err(e),
                _ => {}
            }
        }
        Err("Silent VAD replay timed out".into())
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
        let vad_settings = vad_parameters(&paths, &samples)?;
        let first = replay(&worker, &samples)?;
        let held = held_replay(&worker, &samples, false, true)?;
        let held_cancel = held_replay(&worker, &samples, true, true)?;
        let quick_release = immediate_release(&worker, &samples)?;
        let silence = silent_vad(&worker)?;
        let cancelled = cancel(&worker, &samples)?;
        let restart = replay(&worker, &samples)?;
        send(&worker, Command::Switch(ModelKind::WhisperTurboQ5))?;
        let turbo_load = ready(&worker)?;
        let turbo = bench(&worker)?;
        let turbo_held = held_replay(&worker, &samples, false, false)?;
        send(&worker, Command::Switch(ModelKind::XAsr480))?;
        let reload = ready(&worker)?;
        let last = replay(&worker, &samples)?;
        Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,"source":"WAV replay at real-time pace; no microphone capture",
            "passed":vad_settings["passed"]==true&&held["passed"]==true&&held_cancel["passed"]==true&&quick_release&&silence&&turbo_held["passed"]==true&&first["passed"]==true&&cancelled&&restart["passed"]==true&&turbo["passed"]==true&&last["passed"]==true&&first["finals"]==restart["finals"]&&first["finals"]==last["finals"],
            "vad_settings":vad_settings,"held_release":held,"held_cancel":held_cancel,"immediate_release":quick_release,"sensitive_vad_silence":silence,"turbo_held_release":turbo_held,
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

/// Exercise the installed Ctrl hook against our own EDIT; never opens a microphone.
pub fn hotkey_probe(home: &Path) -> Result<PathBuf, String> {
    use crate::hotkey::{self, CtrlHotkey};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        thread,
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::Input::KeyboardAndMouse::*,
    };
    static MENU_REQUESTS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            // Count native menu activation without entering a modal system-menu loop.
            if msg == WM_SYSCOMMAND && w & 0xfff0 == SC_KEYMENU as usize {
                MENU_REQUESTS.fetch_add(1, Ordering::Relaxed);
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
        native_ctrl: u32,
        native_combo: u32,
    }
    fn pump(hook: &CtrlHotkey, counts: &mut Counts, millis: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(millis) {
            unsafe {
                let mut msg = zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    match msg.message {
                        hotkey::CTRL_RELEASE => counts.releases += 1,
                        hotkey::CTRL_CANCEL => counts.cancels += 1,
                        hotkey::CTRL_FAILED => counts.failures += 1,
                        WM_SYSKEYDOWN | WM_KEYDOWN
                            if [
                                VK_CONTROL as usize,
                                VK_LCONTROL as usize,
                                VK_RCONTROL as usize,
                            ]
                            .contains(&msg.wParam) =>
                        {
                            counts.native_ctrl += 1
                        }
                        WM_KEYDOWN if msg.wParam == 0x41 && GetKeyState(VK_CONTROL as i32) < 0 => {
                            counts.native_combo += 1
                        }
                        _ => {}
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
            // A native short Ctrl tap may move focus to this window's menu.
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
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
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
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("FastAsrCtrlProbe");
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
            wide("Fast ASR · Ctrl 快捷键自测（不录音）").as_ptr(),
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
        focus_probe_window(host, edit);
        // Drain activation before installing the hook or sending synthetic keys.
        for _ in 0..30 {
            let mut msg = zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(5));
        }
        let outcome = (|| -> Result<Value, String> {
            if GetForegroundWindow() != host || GetFocus() != edit {
                return Err("Hotkey test could not focus its own input".into());
            }
            let hook = CtrlHotkey::install(host, true)?;
            let mut counts = Counts::default();
            key(host, edit, VK_LCONTROL, false)?;
            pump(&hook, &mut counts, 160);
            key(host, edit, VK_LCONTROL, true)?;
            pump(&hook, &mut counts, 120);
            let short_ctrl = counts.starts == 0 && counts.releases == 0 && counts.native_ctrl > 0;
            let native_before = counts.native_ctrl;
            let menus_before = MENU_REQUESTS.load(Ordering::Relaxed);
            key(host, edit, VK_LCONTROL, false)?;
            pump(&hook, &mut counts, 220);
            let no_early_start = counts.starts == 0;
            pump(&hook, &mut counts, 180);
            let start_after_threshold = counts.starts == 1;
            for _ in 0..3 {
                key(host, edit, VK_LCONTROL, false)?;
                pump(&hook, &mut counts, 30);
            }
            let once_per_hold = counts.starts == 1;
            key(host, edit, VK_LCONTROL, true)?;
            pump(&hook, &mut counts, 120);
            let release = counts.releases == 1;
            let no_menu = counts.native_ctrl == native_before
                && MENU_REQUESTS.load(Ordering::Relaxed) == menus_before;
            key(host, edit, VK_LCONTROL, false)?;
            pump(&hook, &mut counts, 100);
            key(host, edit, 0x41, false)?;
            pump(&hook, &mut counts, 60);
            key(host, edit, 0x41, true)?;
            key(host, edit, VK_LCONTROL, true)?;
            pump(&hook, &mut counts, 120);
            let combo = counts.native_combo > 0 && counts.starts == 1;
            key(host, edit, VK_LCONTROL, false)?;
            pump(&hook, &mut counts, 410);
            key(host, edit, VK_ESCAPE, false)?;
            pump(&hook, &mut counts, 60);
            key(host, edit, VK_ESCAPE, true)?;
            key(host, edit, VK_LCONTROL, true)?;
            pump(&hook, &mut counts, 100);
            let cancel = counts.starts == 2 && counts.cancels == 1 && counts.releases == 1;
            let no_stuck_modifiers = !input::modifiers_down();
            let no_characters = GetWindowTextLengthW(edit) == 0;
            Ok(
                json!({"version":env!("CARGO_PKG_VERSION"),"executable_sha256":executable_sha256()?,
                "source":"Installed Windows low-level Ctrl hook, synthetic tagged keys, owned EDIT; no microphone",
                "passed":short_ctrl&&no_early_start&&start_after_threshold&&once_per_hold&&release&&no_menu&&combo&&cancel&&no_stuck_modifiers&&no_characters&&counts.failures==0,
                "short_ctrl_restored":short_ctrl,"no_start_before_300ms":no_early_start,"starts_after_300ms":start_after_threshold,
                "once_per_hold":once_per_hold,"release_submits":release,"long_ctrl_does_not_activate_menu":no_menu,
                "ctrl_letter_preserves_native_combo":combo,"escape_cancels":cancel,"no_stuck_modifiers":no_stuck_modifiers,
                "no_typed_characters":no_characters,"replay_failures":counts.failures,"counts":{"start":counts.starts,"release":counts.releases,"cancel":counts.cancels,"ctrl":counts.native_ctrl,"combo":counts.native_combo},"trace":hook.diagnostic_trace()}),
            )
        })();
        // Always balance synthetic keys before closing the test window, including errors.
        let _ = key(host, edit, 0x41, true);
        let _ = key(host, edit, VK_ESCAPE, true);
        let _ = key(host, edit, VK_LCONTROL, true);
        DestroyWindow(host);
        let report = home.join("reports/hotkey-probe.json");
        fs::create_dir_all(report.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&report, serde_json::to_vec_pretty(&outcome?).unwrap())
            .map_err(|e| e.to_string())?;
        Ok(report)
    }
}
