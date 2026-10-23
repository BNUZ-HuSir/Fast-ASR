use crate::{
    assets::{self, ModelKind, Paths},
    audio::Recorder,
    config::Settings,
    engine::{Decoder, Segmenter, TextEvent},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};
pub enum Command {
    Start(Settings),
    Stop,
    Cancel,
    Retry,
    Switch(ModelKind),
    Benchmark,
    Quit,
    Replay(Settings, Vec<f32>),
}
#[derive(Debug)]
pub enum Event {
    Loading(String),
    Ready,
    Started,
    Stopped,
    Partial(String),
    Final(String),
    Level(f32),
    Captured(u64),
    Error(String),
    LoadFailed(String),
    Benchmarked(serde_json::Value),
}
pub struct Worker {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
}
enum Task {
    Load(Paths),
    Begin(u32),
    Decode(Vec<f32>),
    Frame(Vec<f32>),
    Finish,
    Stop,
    Benchmark(Paths),
    Quit,
}
enum Reply {
    Ready,
    Text(Vec<TextEvent>),
    Stopped,
    Error(String),
    LoadFailed(String),
    Benchmarked(serde_json::Value),
}
enum Capture {
    Live(Recorder),
    Replay {
        samples: Vec<f32>,
        cursor: usize,
        start: Instant,
    },
}
impl Capture {
    fn drain(&mut self) -> Result<Vec<Vec<f32>>, String> {
        match self {
            Self::Live(rec) => rec.drain(),
            Self::Replay {
                samples,
                cursor,
                start,
            } => {
                let target = ((start.elapsed().as_secs_f64() * 16000.) as usize).min(samples.len());
                let mut chunks = Vec::new();
                while *cursor + 512 <= target || (target == samples.len() && *cursor < target) {
                    let n = (samples.len() - *cursor).min(512);
                    let mut frame = vec![0.; 512];
                    frame[..n].copy_from_slice(&samples[*cursor..*cursor + n]);
                    *cursor += n;
                    chunks.push(frame);
                }
                Ok(chunks)
            }
        }
    }
    fn stop_chunks(&mut self) -> Vec<Vec<f32>> {
        match self {
            Self::Live(rec) => rec.stop_chunks(),
            Self::Replay { .. } => Vec::new(),
        }
    }
    fn sample_count(&self) -> u64 {
        match self {
            Self::Live(rec) => rec.sample_count,
            Self::Replay { cursor, .. } => *cursor as u64,
        }
    }
    fn finished(&self) -> bool {
        matches!(self,Self::Replay{samples,cursor,..} if *cursor==samples.len())
    }
}
fn enqueue(tasks: &Sender<(u64, Task)>, queued: &AtomicUsize, id: u64, task: Task) {
    let count = match &task {
        Task::Decode(a) | Task::Frame(a) => a.len(),
        _ => 0,
    };
    queued.fetch_add(count, Ordering::Relaxed);
    if tasks.send((id, task)).is_err() {
        queued.fetch_sub(count, Ordering::Relaxed);
    }
}
fn feed_chunks(
    chunks: Vec<Vec<f32>>,
    segmenter: &mut Option<Segmenter>,
    tasks: &Sender<(u64, Task)>,
    queued: &AtomicUsize,
    id: u64,
) -> Result<(), String> {
    if let Some(v) = segmenter {
        for chunk in chunks {
            if let Some(audio) = v.feed(&chunk)? {
                enqueue(tasks, queued, id, Task::Decode(audio));
            }
        }
    } else {
        let samples: Vec<_> = chunks.into_iter().flatten().collect();
        if !samples.is_empty() {
            enqueue(tasks, queued, id, Task::Frame(samples));
        }
    }
    Ok(())
}
fn finish_capture(
    capture: &mut Option<Capture>,
    segmenter: &mut Option<Segmenter>,
    tasks: &Sender<(u64, Task)>,
    queued: &AtomicUsize,
    id: u64,
    events: &Sender<Event>,
) -> Result<(), String> {
    if let Some(mut source) = capture.take() {
        feed_chunks(source.stop_chunks(), segmenter, tasks, queued, id)?;
        let _ = events.send(Event::Captured(source.sample_count()));
        if let Some(v) = segmenter {
            if let Some(audio) = v.finish() {
                enqueue(tasks, queued, id, Task::Decode(audio));
            }
        } else {
            enqueue(tasks, queued, id, Task::Finish);
        }
    }
    Ok(())
}
impl Worker {
    pub fn spawn(home: PathBuf, initial_model: ModelKind) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        thread::spawn(move || {
            let generation = Arc::new(AtomicU64::new(1));
            let queued = Arc::new(AtomicUsize::new(0));
            let (tasks, inference) = mpsc::channel::<(u64, Task)>();
            let (replies, results) = mpsc::channel();
            let gen_infer = generation.clone();
            let queue_infer = queued.clone();
            thread::spawn(move || {
                let mut decoder: Option<Decoder> = None;
                while let Ok((id, task)) = inference.recv() {
                    if matches!(task, Task::Quit) {
                        break;
                    }
                    let sample_count = match &task {
                        Task::Decode(a) | Task::Frame(a) => a.len(),
                        _ => 0,
                    };
                    if id != gen_infer.load(Ordering::Acquire) {
                        queue_infer.fetch_sub(sample_count, Ordering::Relaxed);
                        continue;
                    }
                    let reply = match task {
                        Task::Load(paths) => {
                            decoder = None;
                            match Decoder::new(&paths) {
                                Ok(d) => {
                                    decoder = Some(d);
                                    Reply::Ready
                                }
                                Err(e) => Reply::LoadFailed(e),
                            }
                        }
                        Task::Begin(silence) => match decoder
                            .as_mut()
                            .ok_or_else(|| "模型尚未加载".to_string())
                            .and_then(|d| d.begin(silence))
                        {
                            Ok(()) => continue,
                            Err(e) => Reply::Error(e),
                        },
                        Task::Decode(audio) => match decoder
                            .as_mut()
                            .ok_or_else(|| "模型尚未加载".to_string())
                            .and_then(|d| d.transcribe(&audio))
                        {
                            Ok(s) => Reply::Text(if s.is_empty() {
                                Vec::new()
                            } else {
                                vec![TextEvent::Final(s)]
                            }),
                            Err(e) => Reply::Error(e),
                        },
                        Task::Frame(audio) => match decoder
                            .as_mut()
                            .ok_or_else(|| "模型尚未加载".to_string())
                            .and_then(|d| d.feed_stream(&audio))
                        {
                            Ok(v) => Reply::Text(v),
                            Err(e) => Reply::Error(e),
                        },
                        Task::Finish => match decoder
                            .as_mut()
                            .ok_or_else(|| "模型尚未加载".to_string())
                            .and_then(|d| d.finish_stream())
                        {
                            Ok(v) => Reply::Text(v),
                            Err(e) => Reply::Error(e),
                        },
                        Task::Stop => Reply::Stopped,
                        Task::Benchmark(paths) => match decoder
                            .as_mut()
                            .ok_or_else(|| "模型尚未加载".to_string())
                            .and_then(|d| crate::diagnostics::benchmark_decoder(d, &paths))
                        {
                            Ok(v) => Reply::Benchmarked(v),
                            Err(e) => Reply::Error(e),
                        },
                        Task::Quit => break,
                    };
                    queue_infer.fetch_sub(sample_count, Ordering::Relaxed);
                    if replies.send((id, reply)).is_err() {
                        break;
                    }
                }
            });
            let mut selected = initial_model;
            let mut switch_after_stop = None;
            let mut needs_load = true;
            let mut ready = false;
            let mut segmenter: Option<Segmenter> = None;
            let mut capture: Option<Capture> = None;
            let mut stopping = false;
            let mut last_audio = Instant::now();
            let mut level_time = Instant::now();
            let mut count_time = Instant::now();
            loop {
                if needs_load {
                    needs_load = false;
                    ready = false;
                    capture = None;
                    segmenter = None;
                    stopping = false;
                    let id = generation.load(Ordering::Acquire);
                    match assets::ensure(&home, selected, |s| {
                        let _ = events.send(Event::Loading(s));
                    })
                    .and_then(|p| {
                        if !selected.streaming() {
                            segmenter = Some(Segmenter::new(&p)?);
                        }
                        Ok(p)
                    }) {
                        Ok(p) => enqueue(&tasks, &queued, id, Task::Load(p)),
                        Err(e) => {
                            let _ = events.send(Event::LoadFailed(e));
                        }
                    }
                }
                while let Ok((id, reply)) = results.try_recv() {
                    if id != generation.load(Ordering::Acquire) {
                        continue;
                    }
                    match reply {
                        Reply::Ready => {
                            ready = true;
                            let _ = events.send(Event::Ready);
                        }
                        Reply::Text(texts) => {
                            for text in texts {
                                let event = match text {
                                    TextEvent::Partial(s) => Event::Partial(s),
                                    TextEvent::Final(s) => Event::Final(s),
                                };
                                let _ = events.send(event);
                            }
                        }
                        Reply::Stopped => {
                            stopping = false;
                            if let Some(kind) = switch_after_stop.take() {
                                selected = kind;
                                generation.fetch_add(1, Ordering::AcqRel);
                                needs_load = true;
                            } else {
                                let _ = events.send(Event::Stopped);
                            }
                        }
                        Reply::LoadFailed(e) => {
                            ready = false;
                            let _ = events.send(Event::LoadFailed(e));
                        }
                        Reply::Error(e) => {
                            capture = None;
                            stopping = false;
                            generation.fetch_add(1, Ordering::AcqRel);
                            let _ = events.send(Event::Partial(String::new()));
                            let _ = events.send(Event::Error(e));
                            if let Some(kind) = switch_after_stop.take() {
                                selected = kind;
                                needs_load = true;
                            }
                        }
                        Reply::Benchmarked(v) => {
                            let _ = events.send(Event::Benchmarked(v));
                        }
                    }
                }
                if needs_load {
                    continue;
                }
                match commands.recv_timeout(Duration::from_millis(8)) {
                    Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        enqueue(&tasks, &queued, 0, Task::Quit);
                        return;
                    }
                    Ok(Command::Retry) => {
                        generation.fetch_add(1, Ordering::AcqRel);
                        needs_load = true;
                    }
                    Ok(Command::Switch(kind)) => {
                        let id = generation.load(Ordering::Acquire);
                        if let Err(e) = finish_capture(
                            &mut capture,
                            &mut segmenter,
                            &tasks,
                            &queued,
                            id,
                            &events,
                        ) {
                            let _ = events.send(Event::Error(e));
                        }
                        switch_after_stop = Some(kind);
                        stopping = true;
                        enqueue(&tasks, &queued, id, Task::Stop);
                    }
                    Ok(command @ (Command::Start(_) | Command::Replay(..)))
                        if ready
                            && capture.is_none()
                            && !stopping
                            && switch_after_stop.is_none() =>
                    {
                        let (settings, source) = match command {
                            Command::Start(settings) => {
                                let source = Recorder::open(settings.microphone).map(Capture::Live);
                                (settings, source)
                            }
                            Command::Replay(settings, samples) => (
                                settings,
                                Ok(Capture::Replay {
                                    samples,
                                    cursor: 0,
                                    start: Instant::now(),
                                }),
                            ),
                            _ => unreachable!(),
                        };
                        match source {
                            Ok(source) => {
                                if let Some(v) = segmenter.as_mut() {
                                    v.reset(settings.silence_ms);
                                }
                                enqueue(
                                    &tasks,
                                    &queued,
                                    generation.load(Ordering::Acquire),
                                    Task::Begin(settings.silence_ms),
                                );
                                capture = Some(source);
                                last_audio = Instant::now();
                                let _ = events.send(Event::Started);
                            }
                            Err(e) => {
                                let _ = events.send(Event::Error(e));
                            }
                        }
                    }
                    Ok(Command::Stop) => {
                        let id = generation.load(Ordering::Acquire);
                        if let Err(e) = finish_capture(
                            &mut capture,
                            &mut segmenter,
                            &tasks,
                            &queued,
                            id,
                            &events,
                        ) {
                            let _ = events.send(Event::Error(e));
                        }
                        stopping = true;
                        enqueue(&tasks, &queued, id, Task::Stop);
                    }
                    Ok(Command::Cancel) => {
                        capture = None;
                        stopping = false;
                        switch_after_stop = None;
                        generation.fetch_add(1, Ordering::AcqRel);
                        if let Some(v) = segmenter.as_mut() {
                            v.reset(600);
                        }
                        let _ = events.send(Event::Partial(String::new()));
                        let _ = events.send(Event::Stopped);
                    }
                    Ok(Command::Benchmark)
                        if ready
                            && capture.is_none()
                            && !stopping
                            && switch_after_stop.is_none() =>
                    {
                        enqueue(
                            &tasks,
                            &queued,
                            generation.load(Ordering::Acquire),
                            Task::Benchmark(Paths::resolve(&home, selected)),
                        );
                    }
                    _ => {}
                }
                let mut failure = None;
                let mut ended = false;
                if let Some(source) = capture.as_mut() {
                    match source.drain() {
                        Ok(chunks) => {
                            if !chunks.is_empty() {
                                last_audio = Instant::now();
                            }
                            if level_time.elapsed() >= Duration::from_millis(70)
                                && !chunks.is_empty()
                            {
                                let level =
                                    chunks.iter().flatten().fold(0f32, |a, x| a.max(x.abs()));
                                let _ = events.send(Event::Level(level));
                                level_time = Instant::now();
                            }
                            if let Err(e) = feed_chunks(
                                chunks,
                                &mut segmenter,
                                &tasks,
                                &queued,
                                generation.load(Ordering::Acquire),
                            ) {
                                failure = Some(e);
                            }
                        }
                        Err(e) => failure = Some(e),
                    }
                    if count_time.elapsed() >= Duration::from_millis(200) {
                        let _ = events.send(Event::Captured(source.sample_count()));
                        count_time = Instant::now();
                    }
                    ended = source.finished();
                    if last_audio.elapsed() > Duration::from_secs(4) {
                        failure = Some("麦克风没有返回音频，请检查设备连接与权限".into());
                    }
                }
                if let Some(e) = failure {
                    capture = None;
                    generation.fetch_add(1, Ordering::AcqRel);
                    let _ = events.send(Event::Partial(String::new()));
                    let _ = events.send(Event::Error(e));
                }
                let limit = 16000 * if selected.streaming() { 10 } else { 90 };
                let overloaded = capture.is_some() && queued.load(Ordering::Relaxed) >= limit;
                if ended || overloaded {
                    let id = generation.load(Ordering::Acquire);
                    if let Err(e) =
                        finish_capture(&mut capture, &mut segmenter, &tasks, &queued, id, &events)
                    {
                        let _ = events.send(Event::Error(e));
                    }
                    stopping = true;
                    enqueue(&tasks, &queued, id, Task::Stop);
                    if overloaded {
                        let _ = events.send(Event::Error(
                            "识别速度暂时跟不上，已停止录音并保留已有文字，请等待收尾".into(),
                        ));
                    }
                }
            }
        });
        Self { tx, rx }
    }
}
