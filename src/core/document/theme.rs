//! Portable application-theme schema and built-in values. Files are a native
//! persistence mechanism; these defaults and validators never read a file.
use super::{code_style, DocumentStyleAssignment, StyleSheet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_THEME_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ThemeColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl ThemeColor {
    fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
    fn valid(self) -> bool {
        [self.red, self.green, self.blue, self.alpha]
            .into_iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub foreground: ThemeColor,
    pub background: ThemeColor,
    pub caret: ThemeColor,
    pub selection: ThemeColor,
    pub status_foreground: ThemeColor,
    pub status_background: ThemeColor,
    pub status_font_family: String,
    pub status_font_size: f64,
}

impl Appearance {
    fn preset(paper: bool) -> Self {
        let rgb = |r, g, b| ThemeColor::new(r, g, b, 1.0);
        if paper {
            Self {
                foreground: rgb(0.08, 0.09, 0.11),
                background: rgb(1.0, 1.0, 1.0),
                status_foreground: rgb(0.88, 0.90, 0.93),
                status_background: rgb(0.12, 0.13, 0.15),
                caret: rgb(0.06, 0.24, 0.49),
                selection: ThemeColor::new(0.12, 0.39, 0.73, 0.28),
                status_font_family: "System".into(),
                status_font_size: 11.0,
            }
        } else {
            Self {
                foreground: rgb(0.90, 0.93, 0.98),
                background: rgb(0.035, 0.085, 0.17),
                status_foreground: rgb(0.68, 0.78, 0.91),
                status_background: rgb(0.02, 0.055, 0.12),
                caret: rgb(0.76, 0.86, 1.0),
                selection: ThemeColor::new(0.39, 0.65, 1.0, 0.38),
                status_font_family: "System".into(),
                status_font_size: 11.0,
            }
        }
    }
    fn validate(&self) -> Result<(), String> {
        if ![
            self.foreground,
            self.background,
            self.caret,
            self.selection,
            self.status_foreground,
            self.status_background,
        ]
        .into_iter()
        .all(ThemeColor::valid)
        {
            return Err("Theme colors must be finite sRGB components in 0...1".into());
        }
        if !self.status_font_size.is_finite()
            || !(8.0..=32.0).contains(&self.status_font_size)
            || self.status_font_family.is_empty()
            || self.status_font_family.chars().count() >= 256
            || self.status_font_family.chars().any(char::is_control)
        {
            return Err("Invalid status font".into());
        }
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Styles {
    #[serde(
        default,
        deserialize_with = "present_style",
        skip_serializing_if = "Option::is_none"
    )]
    text: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present_style",
        skip_serializing_if = "Option::is_none"
    )]
    markdown: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present_style",
        skip_serializing_if = "Option::is_none"
    )]
    code: Option<Value>,
}

fn present_style<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    // Absence means built-in; an explicit null is a malformed stylesheet.
    Value::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
struct ThemeFile {
    version: u32,
    theme: Appearance,
    #[serde(default)]
    styles: Styles,
}

/// Preset zero is Midnight (the built-in Default); preset one is Paper.
pub fn default_json(preset: u32) -> Result<Vec<u8>, String> {
    if preset > 1 {
        return Err("Unknown theme preset".into());
    }
    let defaults = StyleSheet::default()
        .default_configuration_json(&DocumentStyleAssignment::new("Paragraph".into()))
        .map_err(|error| error.to_string())?;
    let mut defaults: Value =
        serde_json::from_slice(&defaults).map_err(|error| error.to_string())?;
    // The preset itself is portable; native resolvers map this shared generic
    // family to their system UI font. Document defaults remain platform-native.
    if let Some(blocks) = defaults["block_styles"].as_array_mut() {
        for block in blocks {
            if block["id"] == "Paragraph" {
                block["character"]["font_families"] = serde_json::json!(["system-ui"]);
            }
        }
    }
    // Both sheet schemas serialize the same normalized style records. A full
    // Code v3 image keeps preset files independent of future generated defaults.
    let mut code: Value = serde_json::from_slice(
        &code_style::default_sheet()
            .default_configuration_json(&DocumentStyleAssignment::new("Paragraph".into()))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    code["version"] = 3.into();
    code["suppressed_character_ids"] = Value::Array(Vec::new());
    serde_json::to_vec_pretty(&ThemeFile {
        version: 1,
        theme: Appearance::preset(preset == 1),
        styles: Styles {
            text: Some(defaults.clone()),
            markdown: Some(defaults.clone()),
            code: Some(code),
        },
    })
    .map_err(|error| error.to_string())
}

/// Validate atomically without installing a process or document style sheet.
/// Omitted style families mean their built-in defaults.
pub fn validate_json(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_THEME_BYTES {
        return Err("Theme exceeds 20 MiB".into());
    }
    let file: ThemeFile = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if file.version != 1 {
        return Err("Unsupported theme version".into());
    }
    file.theme.validate()?;
    for style in [file.styles.text, file.styles.markdown]
        .into_iter()
        .flatten()
    {
        let bytes = serde_json::to_vec(&style).map_err(|error| error.to_string())?;
        let (_, diagnostics) = StyleSheet::default()
            .with_default_json(&bytes)
            .map_err(|error| error.to_string())?;
        if !diagnostics.is_empty() {
            return Err(diagnostics.join(" "));
        }
    }
    if let Some(code) = file.styles.code {
        code_style::parse_json(&serde_json::to_vec(&code).map_err(|error| error.to_string())?)?;
    }
    Ok(())
}

/// A theme's display name is also its portable filename stem. Native stores
/// additionally reject case-insensitive collisions with existing themes.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.chars().count() > 32
        || name.chars().all(|ch| ch == '.')
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
    {
        return Err("Use 1–32 characters without filename separators, control characters, or a trailing dot or space".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if name.eq_ignore_ascii_case("Default")
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("This name is reserved".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_and_packaged_presets_are_valid_and_midnight_matches_default() {
        for (preset, name) in [(0, "Midnight"), (1, "Paper")] {
            let bytes = default_json(preset).unwrap();
            validate_json(&bytes).unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(!value["styles"]["code"]["character_styles"]
                .as_array()
                .unwrap()
                .is_empty());
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("assets/themes/{name}.json"));
            let packaged = std::fs::read(path).unwrap();
            validate_json(&packaged).unwrap();
            // Midnight defines the built-in Default. Other installed presets
            // can be customized independently of the emergency fallbacks.
            if preset == 0 {
                let packaged: Value = serde_json::from_slice(&packaged).unwrap();
                assert_eq!(value, packaged);
            }
        }
    }
    #[test]
    fn themes_require_the_current_code_stylesheet_version() {
        let bytes = default_json(0).unwrap();
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["styles"]["code"]["version"], 3);
        validate_json(&bytes).unwrap();
        for version in [1, 2] {
            value["styles"]["code"]["version"] = version.into();
            assert_eq!(
                validate_json(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
                "Unsupported Code stylesheet"
            );
        }
    }

    #[test]
    fn names_are_portable_and_reserved_names_are_rejected() {
        for name in [
            "Default", "default", "CON", "con.json", "LPT9", "", ".", "...", "a.", "a ", "a/b",
            "a\\b", "a:b", "a\n", "a?",
        ] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        for name in ["Midnight", "Paper", "Custom 2", "Mañana", "COM0"] {
            validate_name(name).unwrap();
        }
        assert!(validate_name(&"x".repeat(33)).is_err());
        validate_name(&"é".repeat(32)).unwrap();
    }
    #[test]
    fn invalid_appearance_and_invalid_style_graphs_are_rejected_without_global_changes() {
        let mut value: Value = serde_json::from_slice(&default_json(0).unwrap()).unwrap();
        value["theme"]["foreground"]["red"] = 1.1.into();
        assert!(validate_json(&serde_json::to_vec(&value).unwrap()).is_err());
        value["theme"]["foreground"]["red"] = 0.9.into();
        value["styles"]["text"]["version"] = 99.into();
        assert!(validate_json(&serde_json::to_vec(&value).unwrap()).is_err());
        value["styles"]["text"] = Value::Null;
        assert!(validate_json(&serde_json::to_vec(&value).unwrap()).is_err());
        value.as_object_mut().unwrap().remove("styles");
        validate_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
