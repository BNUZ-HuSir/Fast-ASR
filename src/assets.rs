use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    #[serde(
        rename = "x-asr-zh-en-480ms",
        alias = "whisper-small-q5",
        alias = "whisper-small-q5_1",
        alias = "whisper-small-f16"
    )]
    XAsr480,
    #[serde(rename = "whisper-large-v3-turbo-q5_0")]
    WhisperTurboQ5,
}
impl ModelKind {
    pub const ALL: [Self; 2] = [Self::XAsr480, Self::WhisperTurboQ5];
    pub fn streaming(self) -> bool {
        self == Self::XAsr480
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::XAsr480 => "X-ASR 中英 · 流式 169 MB",
            Self::WhisperTurboQ5 => "Whisper Turbo Q5 · 574 MB · 较慢",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::XAsr480 => "x-asr-zh-en-480ms",
            Self::WhisperTurboQ5 => "whisper-large-v3-turbo-q5_0",
        }
    }
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|k| k.name() == name)
            .unwrap_or(Self::XAsr480)
    }
    pub fn file_name(self) -> &'static str {
        match self {
            Self::XAsr480 => "xasr-480ms",
            Self::WhisperTurboQ5 => "ggml-large-v3-turbo-q5_0.bin",
        }
    }
}
#[derive(Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Deserialize)]
pub struct WhisperManifest {
    pub repository: String,
    pub revision: String,
    pub files: Vec<ModelFile>,
}
pub fn whisper_manifest() -> WhisperManifest {
    serde_json::from_str(include_str!("../assets/whisper-manifest.json")).unwrap()
}
#[derive(Deserialize)]
pub struct XasrManifest {
    pub directory: String,
    pub files: Vec<ModelFile>,
}
pub fn xasr_manifest() -> XasrManifest {
    serde_json::from_str(include_str!("../assets/xasr-manifest.json")).unwrap()
}
pub const WHISPER_RUNTIME: &str = "whisper-1.9.5-win-x64-cpu";
pub const ONLINE_RUNTIME: &str = "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts";
pub const WHISPER_VAD: &str = "ggml-silero-v5.1.2.bin";
pub struct Paths {
    pub runtime: PathBuf,
    pub online_runtime: PathBuf,
    pub model: PathBuf,
    pub vad: PathBuf,
    pub kind: ModelKind,
    pub samples: PathBuf,
}
impl Paths {
    pub fn resolve(home: &Path, kind: ModelKind) -> Self {
        if home.join("runtime").is_dir() {
            Self {
                runtime: home.join("runtime/whisper"),
                online_runtime: home.join("runtime/sherpa"),
                model: home.join("models").join(kind.file_name()),
                vad: home.join("models/vad").join(WHISPER_VAD),
                kind,
                samples: home.join("samples"),
            }
        } else {
            let vendor = home.join("vendor");
            Self {
                runtime: vendor.join("runtime").join(WHISPER_RUNTIME),
                online_runtime: vendor.join("runtime").join(ONLINE_RUNTIME).join("lib"),
                model: if kind.streaming() {
                    vendor.join("models").join(xasr_manifest().directory)
                } else {
                    vendor.join("models/whisper").join(kind.file_name())
                },
                vad: vendor.join("models/vad").join(WHISPER_VAD),
                kind,
                samples: vendor.join("samples/asr"),
            }
        }
    }
}
fn sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn fetch(
    url: &str,
    path: &Path,
    expected: u64,
    hash: &str,
    status: &impl Fn(String),
) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    if path.is_file()
        && fs::metadata(path).is_ok_and(|m| m.len() == expected)
        && sha256(path)? == hash
    {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    if path.exists() {
        fs::rename(path, path.with_extension(format!("damaged-{stamp}")))
            .map_err(|e| e.to_string())?;
    }
    let part = path.with_extension("download");
    if part.is_file() && fs::metadata(&part).is_ok_and(|m| m.len() > expected) {
        fs::rename(&part, part.with_extension(format!("damaged-{stamp}")))
            .map_err(|e| e.to_string())?;
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .user_agent("Fast-ASR/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut failures = 0;
    while fs::metadata(&part).map(|m| m.len()).unwrap_or(0) < expected {
        let offset = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let end = (offset + 4 * 1024 * 1024).min(expected) - 1;
        let attempt = (|| -> Result<(), String> {
            let mut response = client
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?;
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                let range = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("");
                if !range.starts_with(&format!("bytes {offset}-")) {
                    return Err("服务器返回了不匹配的下载区间".into());
                }
            } else if offset != 0 {
                return Err("服务器暂不支持续传，请使用完整便携包".into());
            }
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&part)
                .map_err(|e| e.to_string())?;
            let mut count = offset;
            let mut buffer = [0u8; 65536];
            loop {
                let max = buffer.len().min((expected - count) as usize);
                if max == 0 {
                    break;
                }
                let n = response
                    .read(&mut buffer[..max])
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                count += n as u64;
            }
            file.flush().map_err(|e| e.to_string())?;
            Ok(())
        })();
        let progress = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        status(format!(
            "准备离线资源… {} / {} MB",
            progress / 1048576,
            expected / 1048576
        ));
        if progress > offset {
            failures = 0;
        } else {
            failures += 1;
        }
        if failures >= 4 {
            return Err(format!(
                "资源下载失败：{}。也可重新解压完整便携包。",
                attempt.err().unwrap_or_else(|| "连接提前关闭".into())
            ));
        }
    }
    if sha256(&part)? != hash {
        fs::rename(&part, part.with_extension(format!("damaged-{stamp}")))
            .map_err(|e| e.to_string())?;
        return Err("资源校验失败，点击麦克风重试，或重新解压完整便携包".into());
    }
    fs::rename(&part, path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn ensure(home: &Path, kind: ModelKind, status: impl Fn(String)) -> Result<Paths, String> {
    let paths = Paths::resolve(home, kind);
    status(format!("正在校验 {}…", kind.label()));
    if kind.streaming() {
        for name in [
            "sherpa-onnx-c-api.dll",
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
        ] {
            if !paths.online_runtime.join(name).is_file() {
                return Err(format!("缺少运行库 {name}，请重新解压完整便携包"));
            }
        }
        for file in xasr_manifest().files {
            let path = paths.model.join(&file.name);
            if !path.is_file() {
                return Err(format!(
                    "缺少 X-ASR 模型 {}，请重新解压完整便携包",
                    file.name
                ));
            }
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() != file.bytes
                || sha256(&path)? != file.sha256
            {
                return Err(format!(
                    "X-ASR 模型 {} 校验失败，请重新解压完整便携包",
                    file.name
                ));
            }
        }
    } else {
        for name in ["whisper.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"] {
            if !paths.runtime.join(name).is_file() {
                return Err(format!("缺少运行库 {name}，请重新解压完整便携包"));
            }
        }
        let manifest = whisper_manifest();
        let file = manifest
            .files
            .iter()
            .find(|f| f.name == kind.file_name())
            .ok_or("模型清单不匹配")?;
        fetch(
            &format!(
                "{}/resolve/{}/{}",
                manifest.repository, manifest.revision, file.name
            ),
            &paths.model,
            file.bytes,
            &file.sha256,
            &status,
        )?;
        fetch(
            "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v5.1.2.bin",
            &paths.vad,
            885098,
            "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
            &status,
        )?;
    }
    status(format!("正在加载 {}…", kind.label()));
    Ok(paths)
}
