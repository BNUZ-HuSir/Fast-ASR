use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: crate::assets::ModelKind,
    pub microphone: u32,
    pub silence_ms: u32,
    pub hotkey: bool,
    pub hotkey_hold: bool,
    pub vad_threshold: f32,
    pub vad_min_speech_ms: u32,
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
            hotkey_hold: true,
            vad_threshold: 0.5,
            vad_min_speech_ms: 128,
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
        if value.get("hotkey_hold").is_none() && s.hotkey {
            s.hotkey_hold = false;
        }
        s.normalize();
        s
    }
    pub fn normalize(&mut self) {
        self.silence_ms = self.silence_ms.clamp(350, 1500);
        self.vad_threshold = if self.vad_threshold.is_finite() {
            self.vad_threshold.clamp(0.2, 0.8)
        } else {
            0.5
        };
        self.vad_min_speech_ms = self.vad_min_speech_ms.clamp(64, 512);
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
    fn legacy_shortcuts_keep_toggle_and_new_settings_keep_hold() {
        assert!(!Settings::from_bytes(br#"{"hotkey":true}"#).hotkey_hold);
        let fresh = Settings::from_bytes(b"{}");
        assert!(!fresh.hotkey);
        assert!(fresh.hotkey_hold);
        assert!(Settings::from_bytes(br#"{"hotkey":true,"hotkey_hold":true}"#).hotkey_hold);
    }
    #[test]
    fn vad_limits_and_corrupt_settings_are_safe() {
        let mut s = Settings {
            silence_ms: 0,
            vad_threshold: f32::NAN,
            vad_min_speech_ms: u32::MAX,
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(
            (s.silence_ms, s.vad_threshold, s.vad_min_speech_ms),
            (350, 0.5, 512)
        );
        s.vad_threshold = 2.0;
        s.vad_min_speech_ms = 1;
        s.normalize();
        assert_eq!((s.vad_threshold, s.vad_min_speech_ms), (0.8, 64));
        assert_eq!(Settings::from_bytes(b"broken").vad_threshold, 0.5);
    }
    #[test]
    fn vad_and_shortcut_preferences_roundtrip() {
        let original = Settings {
            hotkey: true,
            hotkey_hold: true,
            vad_threshold: 0.3,
            vad_min_speech_ms: 256,
            silence_ms: 850,
            ..Settings::default()
        };
        let result = Settings::from_bytes(&serde_json::to_vec(&original).unwrap());
        assert!(result.hotkey && result.hotkey_hold);
        assert_eq!(
            (
                result.vad_threshold,
                result.vad_min_speech_ms,
                result.silence_ms
            ),
            (0.3, 256, 850)
        );
    }
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
