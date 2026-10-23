use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::Media::Audio::*;

pub fn devices() -> Vec<(u32, String)> {
    let mut result = vec![(WAVE_MAPPER, "系统默认麦克风".into())];
    unsafe {
        for id in 0..waveInGetNumDevs() {
            let mut caps: WAVEINCAPSW = zeroed();
            if waveInGetDevCapsW(id as usize, &mut caps, size_of::<WAVEINCAPSW>() as u32) == 0 {
                let name = caps.szPname;
                let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                result.push((id, String::from_utf16_lossy(&name[..n])));
            }
        }
    }
    result
}
pub struct Recorder {
    handle: HWAVEIN,
    headers: Vec<WAVEHDR>,
    // WinMM retains raw pointers: each allocation must remain stable.
    #[allow(clippy::vec_box)]
    buffers: Vec<Box<[i16; 512]>>,
    pub sample_count: u64,
    next: usize,
}
impl Recorder {
    pub fn open(device: u32) -> Result<Self, String> {
        unsafe {
            let format = WAVEFORMATEX {
                wFormatTag: 1,
                nChannels: 1,
                nSamplesPerSec: 16000,
                nAvgBytesPerSec: 32000,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let mut handle = null_mut();
            let code = waveInOpen(&mut handle, device, &format, 0, 0, CALLBACK_NULL);
            if code != 0 {
                return Err(error(code));
            }
            let mut rec = Self {
                handle,
                headers: Vec::with_capacity(16),
                buffers: Vec::with_capacity(16),
                sample_count: 0,
                next: 0,
            };
            for _ in 0..16 {
                rec.buffers.push(Box::new([0; 512]));
                let mut h: WAVEHDR = zeroed();
                h.lpData = rec.buffers.last_mut().unwrap().as_mut_ptr().cast();
                h.dwBufferLength = 1024;
                rec.headers.push(h);
            }
            for h in &mut rec.headers {
                let code = waveInPrepareHeader(handle, h, size_of::<WAVEHDR>() as u32);
                if code != 0 {
                    return Err(error(code));
                }
                let code = waveInAddBuffer(handle, h, size_of::<WAVEHDR>() as u32);
                if code != 0 {
                    return Err(error(code));
                }
            }
            let code = waveInStart(handle);
            if code != 0 {
                return Err(error(code));
            }
            Ok(rec)
        }
    }
    pub fn drain(&mut self) -> Result<Vec<Vec<f32>>, String> {
        let mut chunks = Vec::new();
        for _ in 0..self.headers.len() {
            let h = &mut self.headers[self.next];
            // The WinMM driver writes these fields asynchronously.
            let flags = unsafe { std::ptr::read_volatile(std::ptr::addr_of!(h.dwFlags)) };
            if flags & WHDR_DONE == 0 {
                break;
            }
            let count = (h.dwBytesRecorded / 2) as usize;
            if count == 512 {
                let pcm = unsafe { std::slice::from_raw_parts(h.lpData.cast::<i16>(), count) };
                chunks.push(pcm.iter().map(|&s| s as f32 / 32768.).collect());
                self.sample_count += count as u64;
            }
            h.dwBytesRecorded = 0;
            let code = unsafe { waveInAddBuffer(self.handle, h, size_of::<WAVEHDR>() as u32) };
            if code != 0 {
                return Err(error(code));
            }
            self.next = (self.next + 1) % self.headers.len();
        }
        Ok(chunks)
    }
    pub fn stop_chunks(&mut self) -> Vec<Vec<f32>> {
        unsafe {
            waveInStop(self.handle);
            waveInReset(self.handle);
        }
        let mut result = Vec::new();
        for i in 0..self.headers.len() {
            let h = &self.headers[(self.next + i) % self.headers.len()];
            let count = (h.dwBytesRecorded / 2) as usize;
            if count > 0 {
                let mut chunk = vec![0.; 512];
                let pcm =
                    unsafe { std::slice::from_raw_parts(h.lpData.cast::<i16>(), count.min(512)) };
                for (out, &value) in chunk.iter_mut().zip(pcm) {
                    *out = value as f32 / 32768.;
                }
                result.push(chunk);
            }
        }
        result
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe {
            waveInStop(self.handle);
            waveInReset(self.handle);
            for h in &mut self.headers {
                waveInUnprepareHeader(self.handle, h, size_of::<WAVEHDR>() as u32);
            }
            waveInClose(self.handle);
        }
    }
}
fn error(code: u32) -> String {
    let mut text = [0u16; 256];
    unsafe {
        waveInGetErrorTextW(code, text.as_mut_ptr(), text.len() as u32);
    }
    let n = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    format!(
        "麦克风不可用：{}（{}）。请检查设备与 Windows 麦克风权限。",
        String::from_utf16_lossy(&text[..n]),
        code
    )
}
