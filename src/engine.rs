use crate::assets::Paths;
use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
};
unsafe extern "C" {
    fn fa_error() -> *const c_char;
    fn fa_model_create(runtime: *const c_char, model: *const c_char, threads: i32) -> *mut c_void;
    fn fa_model_destroy(ptr: *mut c_void);
    fn fa_transcribe(
        ptr: *mut c_void,
        samples: *const f32,
        n: i32,
        out: *mut c_char,
        cap: i32,
    ) -> i32;
    fn fa_vad_create(runtime: *const c_char, model: *const c_char) -> *mut c_void;
    fn fa_vad_destroy(ptr: *mut c_void);
    fn fa_vad_reset(ptr: *mut c_void);
    fn fa_vad_probability(ptr: *mut c_void, samples: *const f32, n: i32) -> f32;
    fn fax_error() -> *const c_char;
    fn fax_create(runtime: *const c_char, model: *const c_char, threads: i32) -> *mut c_void;
    fn fax_destroy(ptr: *mut c_void);
    fn fax_reset(ptr: *mut c_void, silence_ms: i32) -> i32;
    fn fax_feed(
        ptr: *mut c_void,
        samples: *const f32,
        n: i32,
        finish: i32,
        out: *mut c_char,
        cap: i32,
    ) -> i32;
}
fn path(p: &Path) -> CString {
    CString::new(p.to_string_lossy().as_bytes()).unwrap()
}
fn error() -> String {
    unsafe { CStr::from_ptr(fa_error()).to_string_lossy().into_owned() }
}
fn online_error() -> String {
    unsafe { CStr::from_ptr(fax_error()).to_string_lossy().into_owned() }
}
#[derive(serde::Deserialize)]
struct OnlineOutput {
    finals: Vec<String>,
    partial: String,
}
pub struct Decoder {
    handle: *mut c_void,
    online: bool,
    partial: String,
}
impl Decoder {
    pub fn new(paths: &Paths) -> Result<Self, String> {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 6) as i32)
            .unwrap_or(4);
        let online = paths.kind.streaming();
        let handle = unsafe {
            if online {
                fax_create(
                    path(&paths.online_runtime).as_ptr(),
                    path(&paths.model).as_ptr(),
                    threads,
                )
            } else {
                fa_model_create(
                    path(&paths.runtime).as_ptr(),
                    path(&paths.model).as_ptr(),
                    threads,
                )
            }
        };
        if handle.is_null() {
            Err(if online { online_error() } else { error() })
        } else {
            Ok(Self {
                handle,
                online,
                partial: String::new(),
            })
        }
    }
    pub fn streaming(&self) -> bool {
        self.online
    }
    pub fn begin(&mut self, silence_ms: u32) -> Result<(), String> {
        self.partial.clear();
        if self.online && unsafe { fax_reset(self.handle, silence_ms as i32) } == 0 {
            return Err(online_error());
        }
        Ok(())
    }
    fn online_events(&mut self, samples: &[f32], finish: bool) -> Result<Vec<TextEvent>, String> {
        let mut buffer = vec![0i8; 65536];
        if unsafe {
            fax_feed(
                self.handle,
                samples.as_ptr(),
                samples.len() as i32,
                finish as i32,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        } == 0
        {
            return Err(online_error());
        }
        let bytes = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
        let result: OnlineOutput = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let had_final = !result.finals.is_empty();
        let mut events: Vec<_> = result
            .finals
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(TextEvent::Final)
            .collect();
        if had_final || self.partial != result.partial {
            self.partial = result.partial.clone();
            events.push(TextEvent::Partial(result.partial));
        }
        Ok(events)
    }
    pub fn feed_stream(&mut self, samples: &[f32]) -> Result<Vec<TextEvent>, String> {
        self.online_events(samples, false)
    }
    pub fn finish_stream(&mut self) -> Result<Vec<TextEvent>, String> {
        self.online_events(&[], true)
    }
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        if self.online {
            self.begin(600)?;
            let mut texts = Vec::new();
            for chunk in samples.chunks(512) {
                for event in self.feed_stream(chunk)? {
                    if let TextEvent::Final(s) = event {
                        texts.push(s);
                    }
                }
            }
            for event in self.finish_stream()? {
                if let TextEvent::Final(s) = event {
                    texts.push(s);
                }
            }
            return Ok(texts.join(" "));
        }
        let mut out = vec![0i8; 65536];
        let mut padded = Vec::new();
        let audio = if samples.len() < 16000 {
            padded.extend_from_slice(samples);
            padded.resize(16000, 0.);
            &padded
        } else {
            samples
        };
        if unsafe {
            fa_transcribe(
                self.handle,
                audio.as_ptr(),
                audio.len() as i32,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        } == 0
        {
            return Err(error());
        }
        Ok(unsafe { CStr::from_ptr(out.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string())
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if self.online {
                fax_destroy(self.handle)
            } else {
                fa_model_destroy(self.handle)
            }
        }
    }
}
/// 16 kHz / 512 sample frames. VAD runs on capture thread; model inference never blocks it.
pub struct Segmenter {
    pub streaming: bool,
    handle: *mut c_void,
    pre: VecDeque<f32>,
    audio: Vec<f32>,
    speech_frames: usize,
    quiet_frames: usize,
    silence_frames: usize,
    active: bool,
    threshold: f32,
    min_speech_frames: usize,
    emitted: usize,
}
impl Segmenter {
    pub fn new(paths: &Paths) -> Result<Self, String> {
        let handle =
            unsafe { fa_vad_create(path(&paths.runtime).as_ptr(), path(&paths.vad).as_ptr()) };
        if handle.is_null() {
            return Err(error());
        }
        Ok(Self {
            streaming: paths.kind.streaming(),
            handle,
            pre: VecDeque::new(),
            audio: Vec::new(),
            speech_frames: 0,
            quiet_frames: 0,
            silence_frames: 19,
            active: false,
            threshold: 0.5,
            min_speech_frames: 4,
            emitted: 0,
        })
    }
    pub fn configure(&mut self, settings: &crate::config::Settings) {
        self.threshold = settings.vad_threshold;
        self.min_speech_frames = (settings.vad_min_speech_ms as usize).div_ceil(32);
        self.reset(settings.silence_ms);
    }
    pub fn reset(&mut self, silence_ms: u32) {
        self.emitted = 0;
        self.pre.clear();
        self.audio.clear();
        self.speech_frames = 0;
        self.quiet_frames = 0;
        self.active = false;
        self.silence_frames = (silence_ms as usize).div_ceil(32).max(4);
        unsafe { fa_vad_reset(self.handle) };
    }
    fn advance(&mut self, samples: &[f32]) -> Result<bool, String> {
        if samples.len() != 512 {
            return Err("音频帧长度错误".into());
        }
        let probability = unsafe { fa_vad_probability(self.handle, samples.as_ptr(), 512) };
        if probability < 0. {
            return Err(error());
        }
        let speech = probability
            >= if self.active {
                (self.threshold - 0.15).max(0.05)
            } else {
                self.threshold
            };
        if !self.active {
            self.pre.extend(samples);
            while self.pre.len() > 5120 {
                self.pre.pop_front();
            }
            if speech {
                self.active = true;
                self.audio.extend(self.pre.drain(..));
                self.speech_frames = 1;
                self.quiet_frames = 0;
            }
        } else {
            self.audio.extend_from_slice(samples);
            if speech {
                self.speech_frames += 1;
                self.quiet_frames = 0;
            } else {
                self.quiet_frames += 1;
            }
            // Bound latency and memory during continuous speech, without overlapping committed text.
            if self.quiet_frames >= self.silence_frames || self.audio.len() >= 16000 * 12 {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn feed(&mut self, samples: &[f32]) -> Result<Option<Vec<f32>>, String> {
        if self.advance(samples)? {
            Ok(self.finish())
        } else {
            Ok(None)
        }
    }
    pub fn feed_stream(&mut self, samples: &[f32]) -> Result<(Vec<f32>, bool), String> {
        let ended = self.advance(samples)?;
        let output = self.unsent();
        if ended {
            let _ = self.finish();
        }
        Ok((output, ended))
    }
    fn unsent(&mut self) -> Vec<f32> {
        if self.speech_frames < self.min_speech_frames {
            return Vec::new();
        }
        let output = self.audio[self.emitted..].to_vec();
        self.emitted = self.audio.len();
        output
    }
    pub fn finish_stream(&mut self) -> Vec<f32> {
        let output = self.unsent();
        let _ = self.finish();
        output
    }
    pub fn finish(&mut self) -> Option<Vec<f32>> {
        let valid = self.active && self.speech_frames >= self.min_speech_frames;
        self.emitted = 0;
        let audio = std::mem::take(&mut self.audio);
        self.active = false;
        self.speech_frames = 0;
        self.quiet_frames = 0;
        self.pre.clear();
        // Preserve LSTM continuity across adjacent chunks; reset at explicit recording boundaries.
        if valid { Some(audio) } else { None }
    }
}
impl Drop for Segmenter {
    fn drop(&mut self) {
        unsafe { fa_vad_destroy(self.handle) }
    }
}
pub struct Engine {
    pub decoder: Decoder,
    pub segmenter: Option<Segmenter>,
}
#[derive(Debug)]
pub enum TextEvent {
    Partial(String),
    Final(String),
}
impl Engine {
    pub fn new(paths: &Paths) -> Result<Self, String> {
        Ok(Self {
            decoder: Decoder::new(paths)?,
            segmenter: if paths.kind.streaming() {
                None
            } else {
                Some(Segmenter::new(paths)?)
            },
        })
    }
    pub fn reset(&mut self, silence_ms: u32) -> Result<(), String> {
        if let Some(v) = &mut self.segmenter {
            v.reset(silence_ms);
        }
        self.decoder.begin(silence_ms)
    }
    pub fn feed(&mut self, samples: &[f32]) -> Result<Vec<TextEvent>, String> {
        if self.decoder.streaming() {
            return self.decoder.feed_stream(samples);
        }
        match self.segmenter.as_mut().unwrap().feed(samples)? {
            Some(audio) => Ok(self.decode(&audio)?.into_iter().collect()),
            None => Ok(Vec::new()),
        }
    }
    fn decode(&mut self, audio: &[f32]) -> Result<Option<TextEvent>, String> {
        let text = self.decoder.transcribe(audio)?;
        Ok((!text.is_empty()).then_some(TextEvent::Final(text)))
    }
    pub fn finish(&mut self) -> Result<Option<TextEvent>, String> {
        if self.decoder.streaming() {
            let texts: Vec<_> = self
                .decoder
                .finish_stream()?
                .into_iter()
                .filter_map(|e| match e {
                    TextEvent::Final(s) => Some(s),
                    _ => None,
                })
                .collect();
            return Ok((!texts.is_empty()).then(|| TextEvent::Final(texts.join(" "))));
        }
        match self.segmenter.as_mut().unwrap().finish() {
            Some(audio) => self.decode(&audio),
            None => Ok(None),
        }
    }
}
