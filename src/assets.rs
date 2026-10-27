use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    #[serde(
        rename = "x-asr-zh-en-480ms",
        alias = "whisper-small-q5",
        alias = "whisper-small-q5_1",
        alias = "whisper-small-f16",
        alias = "whisper-large-v3-turbo-q5_0"
    )]
    XAsr480,
}
impl ModelKind {
    pub fn label(self) -> &'static str {
        "X-ASR 中英 · 流式 169 MB"
    }
    pub fn name(self) -> &'static str {
        "x-asr-zh-en-480ms"
    }
    pub fn from_name(_name: &str) -> Self {
        Self::XAsr480
    }
}
#[derive(Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Deserialize)]
pub struct XasrManifest {
    pub directory: String,
    pub files: Vec<ModelFile>,
}
pub fn xasr_manifest() -> XasrManifest {
    serde_json::from_str(include_str!("../assets/xasr-manifest.json")).unwrap()
}
pub const ONLINE_RUNTIME: &str = "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts";
pub struct Paths {
    pub online_runtime: PathBuf,
    pub model: PathBuf,
    pub kind: ModelKind,
    pub samples: PathBuf,
}
impl Paths {
    pub fn resolve(home: &Path, kind: ModelKind) -> Self {
        if home.join("runtime").is_dir() {
            Self {
                online_runtime: home.join("runtime/sherpa"),
                model: home.join("models/xasr-480ms"),
                kind,
                samples: home.join("samples"),
            }
        } else {
            let vendor = home.join("vendor");
            Self {
                online_runtime: vendor.join("runtime").join(ONLINE_RUNTIME).join("lib"),
                model: vendor.join("models").join(xasr_manifest().directory),
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
pub fn ensure(home: &Path, kind: ModelKind, status: impl Fn(String)) -> Result<Paths, String> {
    let paths = Paths::resolve(home, kind);
    status(format!("正在校验 {}…", kind.label()));
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
    status(format!("正在加载 {}…", kind.label()));
    Ok(paths)
}
