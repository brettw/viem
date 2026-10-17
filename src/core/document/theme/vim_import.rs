//! Vim color scheme import. Evaluation follows gVim (`syntax::vim`); this
//! module maps the resolved highlight groups onto a complete theme based on
//! the shipped Paper or Midnight preset. The mapping and its approximations are
//! documented in `docs/vim-theme-import.md`; keep the two in sync.
use super::{validate_json, validate_name, MAX_THEME_BYTES};
use crate::document::code_style;
use crate::document::syntax::vim::colorscheme::{flags, VimBackground, VimColor, VimHighlights, VimResolvedHighlight};
use crate::document::syntax::vim::{evaluate_colorscheme, VimDiagnostic};
use crate::document::{Color, StyleId};
use serde_json::{json, Map, Value};

const PAPER: &[u8] = include_bytes!("../../../../assets/themes/Paper.json");
const MIDNIGHT: &[u8] = include_bytes!("../../../../assets/themes/Midnight.json");

/// Color scheme sources larger than this are not color schemes.
pub const MAX_VIM_COLORSCHEME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct VimThemeImport {
    /// Complete, validated theme JSON.
    pub theme: Vec<u8>,
    /// The built-in preset the theme is based on: "Paper" or "Midnight".
    pub base: &'static str,
    /// Statements and attributes that were skipped while evaluating.
    pub warnings: Vec<VimDiagnostic>,
}

/// Vim standard syntax groups that share their name with a Code style.
const SYNTAX_GROUPS: &[&str] = &[
    "Comment", "Constant", "String", "Character", "Number", "Boolean", "Float", "Identifier", "Function",
    "Statement", "Conditional", "Repeat", "Label", "Operator", "Keyword", "Exception", "PreProc", "Include",
    "Define", "Macro", "PreCondit", "Type", "StorageClass", "Structure", "Typedef", "Special", "SpecialChar",
    "Tag", "Delimiter", "SpecialComment", "Debug", "Error", "Todo", "Added", "Changed", "Removed", "DiffAdd",
    "DiffChange", "DiffDelete",
];

/// Groups Vim's bundled syntax files link to that have no built-in Code style.
/// The Vim provider emits these names, so the import adds styles for them.
const LINK_TARGET_GROUPS: &[&str] = &[
    "Underlined", "Ignore", "Title", "Bold", "Italic", "BoldItalic", "ModeMsg", "MoreMsg", "WarningMsg",
    "ErrorMsg", "Question", "Directory", "NonText", "SpecialKey", "Conceal", "DiffText", "LineNr", "Visual",
    "Folded",
];

/// Tree-sitter capture styles without a same-named Vim group, mapped to the
/// closest standard group. Other captures inherit from their parent style.
const CAPTURE_GROUPS: &[(&str, &str)] = &[
    ("Keyword.import", "Include"),
    ("String.escape", "SpecialChar"),
    ("String.special", "SpecialChar"),
    ("Character.special", "SpecialChar"),
    ("Function.macro", "Macro"),
    ("Number.float", "Float"),
    ("Type.definition", "Typedef"),
    ("Type.qualifier", "StorageClass"),
    ("Attribute", "PreProc"),
    ("Text.uri", "Underlined"),
    ("Markup.heading", "Title"),
    ("Markup.link", "Underlined"),
];

/// The resolved appearance of a style: `None` colors are the theme's text
/// color and a transparent background.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Look {
    foreground: Option<Color>,
    background: Option<Color>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

struct Palette {
    foreground: Color,
    background: Color,
}

impl Palette {
    fn color(&self, color: Option<VimColor>, default: Option<Color>) -> Option<Color> {
        match color {
            Some(VimColor::Rgb([red, green, blue])) => Some(rgb(red, green, blue)),
            Some(VimColor::Foreground) => Some(self.foreground),
            Some(VimColor::Background) => Some(self.background),
            None => default,
        }
    }
    /// Vim's appearance of a group. Reverse video swaps the effective colors,
    /// with Normal's colors standing in for unset ones.
    fn look(&self, resolved: &VimResolvedHighlight) -> Look {
        let gui = resolved.gui;
        let mut look = Look {
            foreground: self.color(gui.foreground, None),
            background: self.color(gui.background, None),
            bold: gui.flags & flags::BOLD != 0,
            italic: gui.flags & flags::ITALIC != 0,
            underline: gui.flags & flags::ANY_UNDERLINE != 0,
            strikethrough: gui.flags & flags::STRIKETHROUGH != 0,
        };
        if gui.flags & (flags::REVERSE | flags::STANDOUT) != 0 {
            let foreground = look.foreground.unwrap_or(self.foreground);
            look.foreground = Some(look.background.unwrap_or(self.background));
            look.background = Some(foreground);
        }
        // A Normal-colored foreground is the theme's text color.
        if look.foreground == Some(self.foreground) {
            look.foreground = None;
        }
        look
    }
    /// The colors Vim actually paints: unset colors are Normal's.
    fn painted(&self, look: &Look) -> (Color, Color) {
        (look.foreground.unwrap_or(self.foreground), look.background.unwrap_or(self.background))
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color { red: f32::from(red) / 255.0, green: f32::from(green) / 255.0, blue: f32::from(blue) / 255.0, alpha: 1.0 }
}

fn transparent() -> Color {
    Color { red: 0.0, green: 0.0, blue: 0.0, alpha: 0.0 }
}

fn luminance(color: Color) -> f32 {
    let channel = |value: f32| if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
    0.2126 * channel(color.red) + 0.7152 * channel(color.green) + 0.0722 * channel(color.blue)
}

fn contrast(first: Color, second: Color) -> f32 {
    let (light, dark) = {
        let (a, b) = (luminance(first), luminance(second));
        if a > b { (a, b) } else { (b, a) }
    };
    (light + 0.05) / (dark + 0.05)
}

fn color_json(color: Color) -> Value {
    json!({"red": color.red, "green": color.green, "blue": color.blue, "alpha": color.alpha})
}

fn color_from_json(value: &Value) -> Option<Color> {
    serde_json::from_value(value.clone()).ok()
}

/// Import a Vim color scheme's GUI colors. `name` names diagnostics only.
pub fn import_vim_colorscheme(name: &str, source: &[u8]) -> Result<VimThemeImport, String> {
    if source.len() > MAX_VIM_COLORSCHEME_BYTES {
        return Err("The Vim color scheme is larger than 4 MiB".into());
    }
    let (highlights, warnings) = evaluate_colorscheme(name, source).map_err(|errors| {
        let first = errors.first().map(|error| format!("{}:{}: {}", error.file, error.line, error.message));
        first.unwrap_or_else(|| "The Vim color scheme could not be evaluated".into())
    })?;
    if !highlights.has_highlight_commands() {
        return Err("The file contains no Vim highlight commands".into());
    }
    let normal = highlights.resolve("Normal").map(|resolved| resolved.gui).unwrap_or_default();
    let normal_rgb = |color: Option<VimColor>| match color {
        Some(VimColor::Rgb([red, green, blue])) => Some(rgb(red, green, blue)),
        _ => None,
    };
    // A light canvas (or, without a Normal background, Vim's 'background',
    // whose default is light) selects Paper; otherwise Midnight.
    let light = match normal_rgb(normal.background) {
        Some(background) => luminance(background) > 0.179,
        None => highlights.background() == VimBackground::Light,
    };
    let (base, bytes) = if light { ("Paper", PAPER) } else { ("Midnight", MIDNIGHT) };
    let mut theme: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let base_foreground = color_from_json(&theme["theme"]["foreground"]).ok_or("Invalid built-in theme")?;
    let base_background = color_from_json(&theme["theme"]["background"]).ok_or("Invalid built-in theme")?;
    let palette = Palette {
        foreground: normal_rgb(normal.foreground).unwrap_or(base_foreground),
        background: normal_rgb(normal.background).unwrap_or(base_background),
    };
    let look = |group: &str| highlights.resolve(group).map(|resolved| palette.look(&resolved));

    // Editor appearance.
    let appearance = &mut theme["theme"];
    appearance["foreground"] = color_json(palette.foreground);
    appearance["background"] = color_json(palette.background);
    // The block cursor's color is its background; without one gVim inverts
    // the text under it, drawing the cursor in the text color.
    let caret = look("Cursor").and_then(|cursor| cursor.background).unwrap_or(palette.foreground);
    appearance["caret"] = color_json(caret);
    if let Some(visual) = look("Visual") {
        if let Some(mut selection) = visual.background {
            // Selection paints beneath unchanged text. Where the scheme relies
            // on its own Visual text color, translucency keeps text readable.
            if visual.foreground.is_some() && contrast(palette.foreground, selection) < 3.0 {
                selection.alpha = 0.4;
            }
            appearance["selection"] = color_json(selection);
        }
    }

    let search = look("Search");
    let status = look("StatusLine").map(|status| palette.painted(&status));
    let line_number = look("LineNr").unwrap_or_default();
    let current_line_number = highlights.defined_by_scheme("CursorLineNr").then(|| look("CursorLineNr")).flatten();
    let chrome = Chrome { palette: &palette, search, status, line_number, current_line_number };

    let styles = theme["styles"].as_object_mut().ok_or("Invalid built-in theme")?;
    // Obsolete stylesheet families are not read and would keep stale colors.
    styles.retain(|family, _| matches!(family.as_str(), "text" | "markdown" | "code"));
    for family in ["text", "markdown", "code"] {
        let sheet = styles.get_mut(family).ok_or("Invalid built-in theme")?;
        chrome.apply(sheet, family == "code")?;
    }
    for family in ["text", "markdown"] {
        inherit_document_colors(&mut styles[family])?;
    }
    code(&mut styles["code"], &highlights, &palette)?;

    let theme = serde_json::to_vec_pretty(&theme).map_err(|error| error.to_string())?;
    if theme.len() > MAX_THEME_BYTES {
        return Err("The imported theme exceeds 20 MiB".into());
    }
    validate_json(&theme)?;
    Ok(VimThemeImport { theme, base, warnings })
}

/// Status bar, line numbers and search matches, shared by every family.
struct Chrome<'a> {
    palette: &'a Palette,
    search: Option<Look>,
    status: Option<(Color, Color)>,
    line_number: Look,
    current_line_number: Option<Look>,
}

impl Chrome<'_> {
    fn apply(&self, sheet: &mut Value, code: bool) -> Result<(), String> {
        let blocks = sheet["block_styles"].as_array_mut().ok_or("Invalid built-in stylesheet")?;
        for block in blocks.iter_mut() {
            match block["id"].as_str() {
                Some("Status bar") => {
                    let Some((foreground, background)) = self.status else { continue };
                    block["character"]["foreground"] = color_json(foreground);
                    block["block"]["background"] = color_json(background);
                    // A status bar the canvas color keeps a visible separator.
                    let separator = if contrast(background, self.palette.background) < 1.1 { foreground } else { background };
                    block["block"]["border_top_color"] = color_json(separator);
                }
                Some("Status bar command") => {
                    let Some((foreground, background)) = self.status else { continue };
                    // Viem's command field is the status bar's inverse.
                    block["character"]["foreground"] = color_json(background);
                    block["block"]["background"] = color_json(foreground);
                }
                Some("Builtin line number") => {
                    gutter(block, &self.line_number, &Look::default());
                }
                Some("Builtin current line number") => {
                    // Vim shows CursorLineNr only with 'cursorline'; otherwise
                    // the current number looks like the others.
                    block["character"] = json!({});
                    block["block"] = json!({});
                    if let Some(current) = self.current_line_number {
                        gutter(block, &current, &self.line_number);
                    }
                }
                _ => {}
            }
        }
        let search = self.search;
        let characters = sheet["character_styles"].as_array_mut().ok_or("Invalid built-in stylesheet")?;
        let incremental = StyleId::incremental_match().0;
        let index = characters.iter().position(|style| style["id"] == incremental.as_str());
        let Some(search) = search else { return Ok(()) };
        let mut properties = Map::new();
        properties.insert("background".into(), color_json(search.background.unwrap_or_else(transparent)));
        if let Some(foreground) = search.foreground {
            properties.insert("foreground".into(), color_json(foreground));
        }
        for (key, value) in [("bold", search.bold), ("underline", search.underline), ("strikethrough", search.strikethrough)] {
            if value {
                properties.insert(key.into(), Value::Bool(true));
            }
        }
        if search.italic {
            properties.insert("slant".into(), "Italic".into());
        }
        let entry = json!({"id": incremental, "name": "Incremental match", "based_on": null, "properties": properties});
        match index {
            Some(index) => characters[index] = entry,
            // The Code sheet stores deltas, so its built-in entry may be absent.
            None if code => characters.push(entry),
            None => return Err("Invalid built-in stylesheet".into()),
        }
        Ok(())
    }
}

/// Line-number declarations relative to `parent`.
fn gutter(block: &mut Value, look: &Look, parent: &Look) {
    if look.foreground != parent.foreground {
        block["character"]["foreground"] = look.foreground.map_or(Value::Null, color_json);
    }
    if look.background != parent.background {
        block["block"]["background"] = look.background.map_or(Value::Null, color_json);
    }
    if look.bold != parent.bold {
        block["character"]["bold"] = look.bold.into();
    }
    if look.italic != parent.italic {
        block["character"]["slant"] = if look.italic { "Italic" } else { "Upright" }.into();
    }
}

/// Document styles (headings, links, code, quotes, tables) drop the preset's
/// colors and inherit Normal's; only typography remains. Editor chrome keeps
/// the scheme's colors from `Chrome`.
fn inherit_document_colors(sheet: &mut Value) -> Result<(), String> {
    const CHROME: &[&str] = &["Status bar", "Status bar command", "Builtin line number", "Builtin current line number"];
    let blocks = sheet["block_styles"].as_array_mut().ok_or("Invalid built-in stylesheet")?;
    for block in blocks.iter_mut().filter(|block| !block["id"].as_str().is_some_and(|id| CHROME.contains(&id))) {
        for key in ["foreground", "background"] {
            if let Some(character) = block["character"].as_object_mut() {
                character.remove(key);
            }
        }
        if let Some(declarations) = block["block"].as_object_mut() {
            declarations.retain(|key, _| key != "background" && !(key.starts_with("border_") && key.ends_with("_color")));
        }
    }
    let incremental = StyleId::incremental_match().0;
    let characters = sheet["character_styles"].as_array_mut().ok_or("Invalid built-in stylesheet")?;
    for character in characters.iter_mut().filter(|style| style["id"] != incremental.as_str()) {
        if let Some(properties) = character["properties"].as_object_mut() {
            properties.remove("foreground");
            properties.remove("background");
        }
    }
    Ok(())
}

/// Rebuild every Code syntax style from the scheme. Each style declares only
/// what differs from the look it inherits, so dotted and linked styles keep
/// following their parents when edited later.
fn code(sheet: &mut Value, highlights: &VimHighlights, palette: &Palette) -> Result<(), String> {
    let defaults = code_style::default_sheet();
    let mut entries = Vec::new();
    let mut looks = std::collections::BTreeMap::<StyleId, Look>::new();
    let mut styles = defaults
        .character_styles()
        .filter(|style| !style.id.is_internal_in(crate::document::StyleNamespace::Character))
        .map(|style| {
            let name = defaults.character_style_metadata(&style.id).map(|metadata| metadata.display_name.clone()).unwrap_or_default();
            (style.id.clone(), style.based_on.clone(), name)
        })
        .collect::<Vec<_>>();
    for group in LINK_TARGET_GROUPS {
        styles.push((StyleId(format!("syntax:{group}")), None, (*group).to_owned()));
    }
    // Parents before children.
    let mut ordered = Vec::with_capacity(styles.len());
    while !styles.is_empty() {
        let before = styles.len();
        let (ready, waiting): (Vec<_>, Vec<_>) = styles.into_iter().partition(|(_, parent, _)| {
            parent.as_ref().is_none_or(|parent| ordered.iter().any(|(id, _, _): &(StyleId, Option<StyleId>, String)| id == parent))
        });
        ordered.extend(ready);
        styles = waiting;
        if styles.len() == before {
            return Err("The built-in Code stylesheet has a cyclic style".into());
        }
    }
    for (id, parent, name) in ordered {
        let inherited = parent.as_ref().and_then(|parent| looks.get(parent)).copied().unwrap_or_default();
        // A same-named Vim group, then a Neovim capture group the scheme
        // defines (`@keyword.import`), then the closest standard group.
        let capture = format!("@{}", name.to_ascii_lowercase());
        let group = if SYNTAX_GROUPS.contains(&name.as_str()) || LINK_TARGET_GROUPS.contains(&name.as_str()) {
            Some(name.as_str())
        } else if highlights.defined_by_scheme(&capture) {
            Some(capture.as_str())
        } else {
            CAPTURE_GROUPS.iter().find(|(style, _)| *style == name).map(|(_, group)| *group)
        };
        let resolved = group.and_then(|group| highlights.resolve(group));
        let look = match resolved {
            Some(resolved) => palette.look(&resolved),
            // Captures without a group, and Markup's typographic defaults,
            // keep their built-in emphasis and inherit their colors.
            None => {
                let properties = defaults.character_style(&id).map(|style| style.properties.clone()).unwrap_or_default();
                Look {
                    bold: properties.bold.unwrap_or(inherited.bold),
                    italic: properties.slant.map_or(inherited.italic, |slant| slant != crate::document::FontSlant::Upright),
                    underline: properties.underline.unwrap_or(inherited.underline),
                    strikethrough: properties.strikethrough.unwrap_or(inherited.strikethrough),
                    ..inherited
                }
            }
        };
        looks.insert(id.clone(), look);
        let mut properties = Map::new();
        if look.foreground != inherited.foreground {
            properties.insert("foreground".into(), color_json(look.foreground.unwrap_or(palette.foreground)));
        }
        if look.background != inherited.background {
            properties.insert("background".into(), color_json(look.background.unwrap_or_else(transparent)));
        }
        if look.bold != inherited.bold {
            properties.insert("bold".into(), look.bold.into());
        }
        if look.italic != inherited.italic {
            properties.insert("slant".into(), if look.italic { "Italic" } else { "Upright" }.into());
        }
        if look.underline != inherited.underline {
            properties.insert("underline".into(), look.underline.into());
        }
        if look.strikethrough != inherited.strikethrough {
            properties.insert("strikethrough".into(), look.strikethrough.into());
        }
        // Omit entries identical to generated definitions, as Code exports do.
        let declared: crate::document::CharacterProperties =
            serde_json::from_value(Value::Object(properties.clone())).map_err(|error| error.to_string())?;
        let unchanged = match defaults.character_style(&id) {
            Some(style) => style.properties == declared,
            None => properties.is_empty(),
        };
        if unchanged {
            continue;
        }
        entries.push(json!({"id": id.0, "name": name, "based_on": parent.map(|parent| parent.0), "properties": properties}));
    }
    // Chrome added the internal search style; keep it after the syntax styles.
    let search = sheet["character_styles"]
        .as_array()
        .and_then(|styles| styles.iter().find(|style| style["id"] == StyleId::incremental_match().0.as_str()).cloned());
    entries.extend(search);
    sheet["character_styles"] = Value::Array(entries);
    sheet["suppressed_character_ids"] = Value::Array(Vec::new());
    Ok(())
}

/// A theme name for an imported file stem: filename-safe, at most 32
/// characters, and distinct from `existing` ignoring case, adding " (2)",
/// " (3)", ... as needed.
pub fn unique_theme_name(stem: &str, existing: &[&str]) -> String {
    let mut base = stem
        .chars()
        .map(|ch| if ch.is_control() || "/\\:*?\"<>|".contains(ch) { '_' } else { ch })
        .collect::<String>();
    while base.ends_with(['.', ' ']) {
        base.pop();
    }
    if base.is_empty() || base.chars().all(|ch| ch == '.') {
        base = "Vim theme".into();
    }
    let taken = |name: &str| existing.iter().any(|other| other.to_lowercase() == name.to_lowercase());
    for number in 1..=10_000u32 {
        let suffix = if number == 1 { String::new() } else { format!(" ({number})") };
        let mut stem = base.chars().take(32 - suffix.chars().count()).collect::<String>();
        while stem.ends_with(['.', ' ']) {
            stem.pop();
        }
        let candidate = format!("{stem}{suffix}");
        if validate_name(&candidate).is_ok() && !taken(&candidate) {
            return candidate;
        }
    }
    // Ten thousand themes with one stem: fall back to a unique counter name.
    (10_001u32..).map(|number| format!("Vim theme ({number})")).find(|name| !taken(name)).unwrap_or_default()
}

#[cfg(test)]
#[path = "vim_import_tests.rs"]
mod tests;
