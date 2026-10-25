use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::{normalize_language, system_language};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub keep_ratio: f32,
    pub max_frames: u32,
    pub debayer: bool,
    pub bayer_pattern: String,
    pub white_balance: bool,
    pub auto_crop: bool,
    pub crop_margin: f32,
    pub drizzle: bool,
    pub flatten_strength: f32,
    pub gamma: f32,
    pub sharpen_amount: f32,
    pub sharpen_radius: f32,
    pub solar_stretch: f32,
    pub subtract_background: bool,
    pub black_point: f32,
    pub protect_highlights: bool,
    pub last_file: String,
    pub last_dir: String,
}

impl ProfileSettings {
    pub fn sun() -> Self {
        let mut profile = Self::common();
        profile.white_balance = false;
        profile.flatten_strength = 70.0;
        profile
    }

    pub fn moon() -> Self {
        let mut profile = Self::common();
        profile.white_balance = true;
        profile.flatten_strength = 0.0;
        profile
    }

    fn common() -> Self {
        Self {
            keep_ratio: 10.0,
            max_frames: 0,
            debayer: true,
            bayer_pattern: "AUTO".to_string(),
            white_balance: false,
            auto_crop: true,
            crop_margin: 5.0,
            drizzle: true,
            flatten_strength: 0.0,
            gamma: 1.2,
            sharpen_amount: 2.0,
            sharpen_radius: 1.0,
            solar_stretch: 0.0,
            subtract_background: true,
            black_point: 0.1,
            protect_highlights: true,
            last_file: String::new(),
            last_dir: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub active: String,
    #[serde(default = "system_language")]
    pub language: String,
    pub sun: ProfileSettings,
    pub moon: ProfileSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active: "sun".to_string(),
            language: system_language(),
            sun: ProfileSettings::sun(),
            moon: ProfileSettings::moon(),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&text).unwrap_or_default().prepared()
    }

    fn prepared(mut self) -> Self {
        self.sun.bayer_pattern = sanitize_pattern(&self.sun.bayer_pattern);
        self.moon.bayer_pattern = sanitize_pattern(&self.moon.bayer_pattern);
        if self.active != "moon" {
            self.active = "sun".to_string();
        }
        self.language = if self.language.trim().is_empty() {
            system_language()
        } else {
            normalize_language(&self.language)
        };
        self
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

pub fn sanitize_pattern(pattern: &str) -> String {
    match pattern.trim().to_ascii_uppercase().as_str() {
        "AUTO" => "AUTO".to_string(),
        "GRBG" | "GRGB" => "GRBG".to_string(),
        "GBRG" => "GBRG".to_string(),
        "BGGR" => "BGGR".to_string(),
        "RGGB" => "RGGB".to_string(),
        _ => "AUTO".to_string(),
    }
}

pub fn settings_path() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        base.join("dubdsr").join("settings.json")
    } else {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("dubdsr").join("settings.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bayer_becomes_auto() {
        assert_eq!(sanitize_pattern("BGRG"), "AUTO");
        assert_eq!(sanitize_pattern("GRGB"), "GRBG");
        assert_eq!(sanitize_pattern("rggb"), "RGGB");
    }

    #[test]
    fn language_is_kept_and_old_files_still_load() {
        let mut settings = AppSettings::default();
        settings.language = "es".to_string();
        settings.active = "moon".to_string();
        let text = serde_json::to_string(&settings).unwrap();
        let loaded = serde_json::from_str::<AppSettings>(&text).unwrap().prepared();
        assert_eq!(loaded.language, "es");
        assert_eq!(loaded.active, "moon");
        assert_eq!(loaded.sun.flatten_strength, 70.0);

        let mut value = serde_json::to_value(&settings).unwrap();
        value.as_object_mut().unwrap().remove("language");
        let legacy = serde_json::from_value::<AppSettings>(value).unwrap().prepared();
        assert!(matches!(legacy.language.as_str(), "fr" | "en" | "es" | "de"));
        assert_eq!(legacy.active, "moon");
        assert_eq!(legacy.moon.white_balance, true);

        settings.language = "pt".to_string();
        let odd = serde_json::from_str::<AppSettings>(&serde_json::to_string(&settings).unwrap())
            .unwrap()
            .prepared();
        assert_eq!(odd.language, "en");
        assert_eq!(odd.active, "moon");
    }
}
