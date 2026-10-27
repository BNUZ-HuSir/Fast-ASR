use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: crate::assets::ModelKind,
    pub microphone: u32,
    pub hotkey: bool,
    pub hotkey_hold: bool,
    pub topmost: bool,
    pub position: Option<(i32, i32)>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            model: crate::assets::ModelKind::XAsr480,
            microphone: u32::MAX,
            hotkey: true,
            hotkey_hold: true,
            topmost: true,
            position: None,
        }
    }
}
impl Settings {
    pub fn load(home: &Path) -> Self {
        Self::from_bytes(&std::fs::read(home.join("data/settings.json")).unwrap_or_default())
    }
    fn from_bytes(bytes: &[u8]) -> Self {
        let value = serde_json::from_slice::<serde_json::Value>(bytes).unwrap_or_default();
        let mut s: Self = serde_json::from_value(value.clone()).unwrap_or_default();
        // Preserve the existing toggle mode when upgrading older enabled shortcuts.
        if value.get("hotkey_hold").is_none()
            && value.get("hotkey").and_then(serde_json::Value::as_bool) == Some(true)
        {
            s.hotkey_hold = false;
        }
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
    fn first_use_and_invalid_config_enable_ctrl_hold() {
        for bytes in [b"".as_slice(), b"{}", b"broken"] {
            let s = Settings::from_bytes(bytes);
            assert!(s.hotkey && s.hotkey_hold);
        }
    }
    #[test]
    fn upgrading_preserves_explicit_shortcut_preferences() {
        assert!(!Settings::from_bytes(br#"{"hotkey":false,"hotkey_hold":true}"#).hotkey);
        assert!(!Settings::from_bytes(br#"{"hotkey":true}"#).hotkey_hold);
        let held = Settings::from_bytes(br#"{"hotkey":true,"hotkey_hold":true}"#);
        assert!(held.hotkey && held.hotkey_hold);
    }
    #[test]
    fn retired_models_migrate_without_losing_preferences() {
        for model in [
            "whisper-small-q5",
            "whisper-small-q5_1",
            "whisper-small-f16",
            "whisper-large-v3-turbo-q5_0",
        ] {
            let value = serde_json::json!({"model":model,"microphone":2,"hotkey":false,"silence_ms":850,"vad_threshold":0.3,"position":[100,200]});
            let s = Settings::from_bytes(&serde_json::to_vec(&value).unwrap());
            assert_eq!(s.model, crate::assets::ModelKind::XAsr480);
            assert_eq!(s.microphone, 2);
            assert!(!s.hotkey);
            assert_eq!(s.position, Some((100, 200)));
            let saved = serde_json::to_value(&s).unwrap();
            assert!(saved.get("vad_threshold").is_none());
            assert!(saved.get("silence_ms").is_none());
        }
    }
    #[test]
    fn preferences_roundtrip() {
        let s = Settings {
            hotkey: false,
            topmost: false,
            microphone: 3,
            ..Settings::default()
        };
        let result = Settings::from_bytes(&serde_json::to_vec(&s).unwrap());
        assert!(!result.hotkey && !result.topmost);
        assert_eq!(result.microphone, 3);
    }
}
