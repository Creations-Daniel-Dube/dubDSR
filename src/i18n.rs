use std::collections::HashMap;

pub struct Catalog {
    entries: HashMap<String, String>,
}

impl Catalog {
    pub fn load(language: &str) -> Self {
        let source = match language {
            "fr" => include_str!("../locales/fr.ftl"),
            "es" => include_str!("../locales/es.ftl"),
            "de" => include_str!("../locales/de.ftl"),
            _ => include_str!("../locales/en.ftl"),
        };
        let mut entries = HashMap::new();
        for line in source.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(" = ") else {
                continue;
            };
            entries.insert(key.to_string(), value.replace("\\n", "\n"));
        }
        Self { entries }
    }

    pub fn text(&self, key: &str) -> String {
        self.entries.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    pub fn fmt(&self, key: &str, args: &[&str]) -> String {
        let mut value = self.text(key);
        for (index, arg) in args.iter().enumerate() {
            value = value.replace(&format!("{{{}}}", index + 1), arg);
        }
        value
    }
}

pub fn system_language() -> String {
    supported_language(&sys_locale::get_locale().unwrap_or_default())
        .unwrap_or("en")
        .to_string()
}

pub fn normalize_language(language: &str) -> String {
    supported_language(language).unwrap_or("en").to_string()
}

fn supported_language(language: &str) -> Option<&'static str> {
    match language.trim().to_ascii_lowercase().split(['-', '_']).next().unwrap_or("") {
        "fr" => Some("fr"),
        "en" => Some("en"),
        "es" => Some("es"),
        "de" => Some("de"),
        _ => None,
    }
}

pub fn cycle_language(language: &str) -> &'static str {
    match language {
        "fr" => "en",
        "en" => "es",
        "es" => "de",
        _ => "fr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn french_catalog_formats_a_stage() {
        let catalog = Catalog::load("fr");
        assert_eq!(catalog.text("stage.read_avi"), "Lecture AVI");
        assert_eq!(catalog.fmt("gpu.available", &["RTX", "12.0"]), "RTX — 12.0 Go VRAM libre");
    }

    #[test]
    fn language_codes_stay_canonical() {
        assert_eq!(normalize_language("ES"), "es");
        assert_eq!(normalize_language("de-DE"), "de");
        assert_eq!(normalize_language("fr_CA"), "fr");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("pt"), "en");
        assert!(matches!(system_language().as_str(), "fr" | "en" | "es" | "de"));
    }
}
