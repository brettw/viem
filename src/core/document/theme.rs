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
    if preset == 0 {
        // Default must remain the complete shipped Midnight preset, including
        // its independent Text, Markdown, and Code declarations. Embedding the
        // canonical bytes keeps damaged bundles usable without a second copy
        // of the preset's typography and colors or a runtime resource read.
        return Ok(include_bytes!("../../../assets/themes/Midnight.json").to_vec());
    }
    let defaults = |format| -> Result<Value, String> {
        let bytes = StyleSheet::for_format(format)
            .default_configuration_json(&DocumentStyleAssignment::new("Paragraph".into()))
            .map_err(|error| error.to_string())?;
        let mut value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        // Native resolvers map this portable family to their system UI font.
        if let Some(blocks) = value["block_styles"].as_array_mut() {
            for block in blocks {
                if block["id"] == "Paragraph" {
                    block["character"]["font_families"] = serde_json::json!(["system-ui"]);
                }
            }
        }
        Ok(value)
    };
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
            text: Some(defaults(super::Format::PlainText)?),
            markdown: Some(defaults(super::Format::Markdown)?),
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
    for (format, style) in [(super::Format::PlainText, file.styles.text), (super::Format::Markdown, file.styles.markdown)] {
        let Some(style) = style else { continue; };
        let bytes = serde_json::to_vec(&style).map_err(|error| error.to_string())?;
        let (_, diagnostics) = StyleSheet::for_format(format)
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
        fn assert_plain_text_styles(bytes: &[u8]) {
            let value: Value = serde_json::from_slice(bytes).unwrap();
            let text = &value["styles"]["text"];
            let blocks = text["block_styles"].as_array().unwrap();
            let characters = text["character_styles"].as_array().unwrap();
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0]["id"], "Paragraph");
            assert_eq!(characters.len(), 1);
            assert_eq!(characters[0]["id"], "* Incremental match");
        }
        for (preset, name) in [(0, "Midnight"), (1, "Paper")] {
            let bytes = default_json(preset).unwrap();
            validate_json(&bytes).unwrap();
            assert_plain_text_styles(&bytes);
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(!value["styles"]["code"]["character_styles"]
                .as_array()
                .unwrap()
                .is_empty());
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("assets/themes/{name}.json"));
            let packaged = std::fs::read(path).unwrap();
            validate_json(&packaged).unwrap();
            assert_plain_text_styles(&packaged);
            // Midnight defines the built-in Default. Other installed presets
            // can be customized independently of the emergency fallbacks.
            if preset == 0 {
                let packaged: Value = serde_json::from_slice(&packaged).unwrap();
                assert_eq!(value, packaged);
            }
        }
        for name in ["Midnight Mono", "Midnight Proportional", "Typewriter"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("assets/themes/{name}.json"));
            let bytes = std::fs::read(path).unwrap();
            validate_json(&bytes).unwrap();
            assert_plain_text_styles(&bytes);
        }
    }
    #[test]
    fn aggregate_theme_fixture_resolves_family_typography_and_inherited_table_edges() {
        let mut value: Value = serde_json::from_slice(&default_json(0).unwrap()).unwrap();
        for (family, size, weight) in [
            ("text", 13.0, 300_u16),
            ("markdown", 17.0, 450),
            ("code", 19.0, 600),
        ] {
            // Presets are editable examples. Supply every numeric declaration
            // this resolution test relies on instead of freezing their values.
            let blocks = value["styles"][family]["block_styles"]
                .as_array_mut()
                .unwrap();
            let paragraph = blocks
                .iter_mut()
                .find(|block| block["id"] == "Paragraph")
                .unwrap();
            paragraph["character"] = serde_json::json!({
                "font_families": ["Fixture font"], "size": size,
                "weight": weight, "font_axes": {"wght": weight}
            });
            paragraph["block"] = serde_json::json!({
                "line_spacing": {"Multiplier": 1.25}, "margin_top": 11.0, "margin_bottom": 7.0
            });
            if family == "markdown" {
                let cell = blocks
                    .iter_mut()
                    .find(|block| block["id"] == "Table cell")
                    .unwrap();
                cell["based_on"] = "Paragraph".into();
                cell["block"] = serde_json::json!({
                    "padding_top": 6.0, "padding_right": 6.0,
                    "padding_bottom": 6.0, "padding_left": 6.0,
                    "border_top_width": 2.0, "border_bottom_width": 1.0,
                    "border_top_color": {"red": 0.25, "green": 0.5, "blue": 0.75, "alpha": 1.0}
                });
                let header = blocks
                    .iter_mut()
                    .find(|block| block["id"] == "Table header")
                    .unwrap();
                header["based_on"] = "Table cell".into();
                header["block"] = serde_json::json!({"border_bottom_width": 4.0});
            }
            let bytes = serde_json::to_vec(&value["styles"][family]).unwrap();
            let sheet = if family == "code" {
                code_style::parse_json(&bytes).unwrap()
            } else {
                let format = if family == "text" { super::super::Format::PlainText } else { super::super::Format::Markdown };
                let (sheet, diagnostics) = StyleSheet::for_format(format).with_default_json(&bytes).unwrap();
                assert!(diagnostics.is_empty(), "{diagnostics:?}");
                sheet
            };
            let paragraph = sheet.default_paragraph_style().unwrap();
            assert_eq!(paragraph.character.font_families, ["Fixture font"]);
            assert_eq!(paragraph.character.size, size as f32);
            assert_eq!(paragraph.character.weight, weight);
            assert_eq!(paragraph.character.font_axes["wght"], f32::from(weight));
            assert_eq!(
                paragraph.line_spacing,
                super::super::LineSpacing::Multiplier(1.25)
            );
            assert_eq!((paragraph.margin_top, paragraph.margin_bottom), (11.0, 7.0));
            if family == "markdown" {
                let header = sheet
                    .resolve_paragraph_style(
                        &"Paragraph".into(),
                        &"Table header".into(),
                        None,
                        &Default::default(),
                        &Default::default(),
                    )
                    .unwrap();
                assert_eq!(
                    (
                        header.padding_top,
                        header.padding_right,
                        header.padding_bottom,
                        header.padding_left
                    ),
                    (6.0, 6.0, 6.0, 6.0)
                );
                assert_eq!(
                    (header.border_top_width, header.border_bottom_width),
                    (2.0, 4.0)
                );
                assert_eq!(
                    header.border_top_color,
                    Some(super::super::Color {
                        red: 0.25,
                        green: 0.5,
                        blue: 0.75,
                        alpha: 1.0
                    })
                );
            }
        }
        validate_json(&serde_json::to_vec(&value).unwrap()).unwrap();
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
