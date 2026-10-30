//! Portable config.json schema. Native hosts own paths and atomic file I/O.
//! Normalize only in memory until an explicit settings write; retain unknown keys.
use crate::{
    document::{
        syntax::detection::{validate_filename_associations, FilenameAssociation},
        IndentationOptions,
    },
    layout::{WhitespacePresentationOptions, ZOOM_STOPS},
};
use serde_json::{json, Value};
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

pub fn defaults() -> Value {
    json!({"version":1,
        "view":{"zoomScale":1.0,"margins":{"top":10,"left":10,"bottom":10,"right":10}},
        "editing":{"autoselect":true,"caretHoverEffect":true,"markdownAutodetect":true,"smartQuotes":true,"smartHome":true,
            "markdownFormattedView":true,"textWidth":80,
            "indentation":IndentationOptions::default(),"whitespacePresentation":WhitespacePresentationOptions::default()},
        "appearance":{"showStatusBar":true},"windows":{"showMenu":true},
        "code":{"filenameAssociations":[]},
        "formattingToolbar":{"plainText":true,"code":true,"markdown":true},"recentDocuments":[]})
}
fn fill(value: &mut Value, defaults: Value) {
    if let (Some(value), Value::Object(defaults)) = (value.as_object_mut(), defaults) {
        for (key, default) in defaults {
            if let Some(current) = value.get_mut(&key) {
                fill(current, default);
            } else {
                value.insert(key, default);
            }
        }
    }
}
fn number(value: &Value, name: &str, min: f64, max: f64, integral: bool) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n >= min && *n <= max && (!integral || n.fract() == 0.0))
        .ok_or_else(|| format!("Invalid {name}"))
}
pub fn normalize(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Configuration must fit within 4 MiB".into());
    }
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid configuration JSON: {e}"))?;
    if !value.is_object() {
        return Err("Configuration must be a JSON object".into());
    }
    if value["version"].as_f64() != Some(1.0) {
        return Err("Unsupported configuration version; expected 1".into());
    }
    for section in [
        "view",
        "editing",
        "appearance",
        "code",
        "windows",
        "formattingToolbar",
    ] {
        if value.get(section).is_some_and(|v| !v.is_object()) {
            return Err(format!("{section} must be an object"));
        }
    }
    for key in ["selectedTheme", "selectedThemeFile"] {
        if value
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return Err(format!("{key} must be a string or null"));
        }
    }
    for (section, keys) in [
        (
            "editing",
            &[
                "autoselect",
                "caretHoverEffect",
                "markdownAutodetect",
                "smartQuotes",
                "smartHome",
                "markdownFormattedView",
            ][..],
        ),
        ("appearance", &["showStatusBar"][..]),
        ("windows", &["showMenu"][..]),
        ("formattingToolbar", &["plainText", "code", "markdown"][..]),
    ] {
        for key in keys {
            if value[section].get(key).is_some_and(|v| !v.is_boolean()) {
                return Err(format!("{key} must be Boolean"));
            }
        }
    }
    if let Some(scale) = value["view"].get("zoomScale") {
        number(
            scale,
            "zoomScale (0.25–5)",
            ZOOM_STOPS[0] as f64,
            ZOOM_STOPS[ZOOM_STOPS.len() - 1] as f64,
            false,
        )?;
    }
    if let Some(margins) = value["view"].get("margins") {
        if !margins.is_object() {
            return Err("View margins must be an object".into());
        }
        for edge in ["top", "left", "bottom", "right"] {
            if let Some(v) = margins.get(edge) {
                number(v, "view margin (0–1000)", 0.0, 1000.0, false)?;
            }
        }
    }
    if let Some(v) = value
        .get_mut("editing")
        .and_then(|editing| editing.get_mut("textWidth"))
    {
        let width = number(v, "textWidth (1–4294967295)", 1.0, u32::MAX as f64, true)?;
        // Native integer readers must also accept an integral JSON spelling such
        // as 72.0, which passes the shared whole-number validation.
        *v = Value::from(width as u32);
    }
    if let Some(frame) = value["windows"].get("documentFrame") {
        let mut values = Vec::new();
        for key in ["x", "y", "width", "height"] {
            values.push(number(
                &frame[key],
                "document window frame",
                -f64::MAX,
                f64::MAX,
                false,
            )?);
        }
        if values[2] <= 0.0
            || values[3] <= 0.0
            || !(values[0] + values[2]).is_finite()
            || !(values[1] + values[3]).is_finite()
        {
            return Err(
                "Document window frame must have finite coordinates and positive dimensions".into(),
            );
        }
    }
    if let Some(recent) = value.get("recentDocuments") {
        let paths = recent
            .as_array()
            .filter(|v| v.len() <= 10)
            .ok_or("Recent documents must contain at most 10 absolute paths")?;
        for path in paths {
            let path = path.as_str().ok_or("Recent documents must be paths")?;
            let drive = path.as_bytes().get(0..3).is_some_and(|b| {
                b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
            });
            if path.len() > 16384
                || path.contains('\0')
                || !(path.starts_with('/') || path.starts_with("\\\\") && path.len() > 2 || drive)
            {
                return Err("Recent documents must use absolute paths of at most 16 KiB".into());
            }
        }
    }
    if let Some(v) = value
        .get_mut("editing")
        .and_then(|editing| editing.get_mut("indentation"))
    {
        let options: IndentationOptions =
            serde_json::from_value(v.clone()).map_err(|e| format!("Invalid indentation: {e}"))?;
        options.validate()?;
        fill(v, serde_json::to_value(options).map_err(|e| e.to_string())?);
    }
    if let Some(v) = value
        .get_mut("editing")
        .and_then(|editing| editing.get_mut("whitespacePresentation"))
    {
        let options: WhitespacePresentationOptions = serde_json::from_value(v.clone())
            .map_err(|e| format!("Invalid whitespace presentation: {e}"))?;
        options.validate().map_err(|e| e.to_string())?;
        fill(v, serde_json::to_value(options).map_err(|e| e.to_string())?);
    }
    if let Some(v) = value["code"].get("filenameAssociations") {
        let options: Vec<FilenameAssociation> = serde_json::from_value(v.clone())
            .map_err(|e| format!("Invalid filename associations: {e}"))?;
        validate_filename_associations(&options)?;
    }
    let mut defaults = defaults();
    // Sparse style declarations do not inherit schema defaults recursively.
    for key in ["indentation", "whitespacePresentation"] {
        if value["editing"].get(key).is_some() {
            defaults["editing"].as_object_mut().unwrap().remove(key);
        }
    }
    fill(&mut value, defaults);
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("Configuration must fit within 4 MiB".into());
    }
    Ok(value)
}
