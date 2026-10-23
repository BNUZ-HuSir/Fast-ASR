use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: crate::assets::ModelKind,
    pub microphone: u32,
    pub silence_ms: u32,
    pub hotkey: bool,
    pub topmost: bool,
    pub position: Option<(i32, i32)>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            model: crate::assets::ModelKind::XAsr480,
            microphone: u32::MAX,
            silence_ms: 600,
            hotkey: false,
            topmost: true,
            position: None,
        }
    }
}
impl Settings {
    pub fn load(home: &Path) -> Self {
        let mut s: Self = std::fs::read(home.join("data/settings.json"))
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        s.silence_ms = s.silence_ms.clamp(350, 1500);
        s
    }
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let dir = home.join("data");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("settings.json"), bytes).map_err(|e| e.to_string())
    }
}
pub fn home() -> PathBuf {
    if let Some(path) = std::env::var_os("FAST_ASR_HOME") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_models_migrate_without_losing_user_preferences() {
        for model in [
            "whisper-small-q5",
            "whisper-small-q5_1",
            "whisper-small-f16",
        ] {
            let json = serde_json::json!({"model":model,"microphone":2,"hotkey":true,"silence_ms":850,"position":[100,200]});
            let settings: Settings = serde_json::from_value(json).unwrap();
            assert_eq!(settings.model, crate::assets::ModelKind::XAsr480);
            assert_eq!(settings.microphone, 2);
            assert!(settings.hotkey);
            assert!(settings.topmost);
            assert_eq!(settings.silence_ms, 850);
            assert_eq!(settings.position, Some((100, 200)));
        }
    }
    #[test]
    fn turbo_choice_survives_save_roundtrip() {
        let original = Settings {
            model: crate::assets::ModelKind::WhisperTurboQ5,
            ..Settings::default()
        };
        let result: Settings =
            serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
        assert_eq!(result.model, original.model);
    }
}
