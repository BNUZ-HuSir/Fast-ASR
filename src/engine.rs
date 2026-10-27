use crate::assets::Paths;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
};
unsafe extern "C" {
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
    partial: String,
}
impl Decoder {
    pub fn new(paths: &Paths) -> Result<Self, String> {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 6) as i32)
            .unwrap_or(4);
        let handle = unsafe {
            fax_create(
                path(&paths.online_runtime).as_ptr(),
                path(&paths.model).as_ptr(),
                threads,
            )
        };
        if handle.is_null() {
            Err(online_error())
        } else {
            Ok(Self {
                handle,
                partial: String::new(),
            })
        }
    }
    pub fn begin(&mut self, silence_ms: u32) -> Result<(), String> {
        self.partial.clear();
        if unsafe { fax_reset(self.handle, silence_ms as i32) } == 0 {
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
        Ok(texts.join(" "))
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { fax_destroy(self.handle) }
    }
}
pub struct Engine {
    pub decoder: Decoder,
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
        })
    }
    pub fn reset(&mut self, silence_ms: u32) -> Result<(), String> {
        self.decoder.begin(silence_ms)
    }
    pub fn feed(&mut self, samples: &[f32]) -> Result<Vec<TextEvent>, String> {
        self.decoder.feed_stream(samples)
    }
    pub fn finish(&mut self) -> Result<Option<TextEvent>, String> {
        let texts: Vec<_> = self
            .decoder
            .finish_stream()?
            .into_iter()
            .filter_map(|e| match e {
                TextEvent::Final(s) => Some(s),
                _ => None,
            })
            .collect();
        Ok((!texts.is_empty()).then(|| TextEvent::Final(texts.join(" "))))
    }
}
