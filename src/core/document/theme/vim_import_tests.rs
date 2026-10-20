use super::*;
use crate::document::{StyleSheet, Format};

fn import(source: &str) -> (Value, &'static str) {
    let imported = import_vim_colorscheme("test.vim", source.as_bytes()).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    (serde_json::from_slice(&imported.theme).unwrap(), imported.base)
}

fn hex(value: &Value) -> String {
    let channel = |key: &str| (value[key].as_f64().unwrap() * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", channel("red"), channel("green"), channel("blue"))
}

fn block<'a>(theme: &'a Value, family: &str, id: &str) -> &'a Value {
    theme["styles"][family]["block_styles"].as_array().unwrap().iter().find(|style| style["id"] == id).unwrap()
}

fn character<'a>(theme: &'a Value, family: &str, id: &str) -> Option<&'a Value> {
    theme["styles"][family]["character_styles"].as_array().unwrap().iter().find(|style| style["id"] == id)
}

const DARK: &str = "set background=dark\nhi clear\nlet g:colors_name = 'dark'\n\
    hi Normal guifg=#d0d0d0 guibg=#101418\nhi Comment guifg=#7f8c8d gui=italic\n\
    hi Statement guifg=#ff7b72 gui=bold\nhi Keyword guifg=#ffa657 gui=NONE\nhi! link Conditional Statement\n\
    hi Constant guifg=#79c0ff\nhi String guifg=#a5d6ff\nhi Visual guibg=#264f78\nhi Cursor guifg=#101418 guibg=#58a6ff\n\
    hi Search guifg=#000000 guibg=#e3b341\nhi StatusLine guifg=#c9d1d9 guibg=#30363d gui=NONE\n\
    hi LineNr guifg=#6e7681\nhi CursorLineNr guifg=#e6edf3 gui=bold\nhi Title guifg=#d2a8ff gui=bold\nhi Todo guifg=#ffffff guibg=#bb0000\n";

#[test]
fn dark_scheme_uses_midnight_and_maps_editor_and_chrome_colors() {
    let (theme, base) = import(DARK);
    assert_eq!(base, "Midnight");
    validate_json(&serde_json::to_vec(&theme).unwrap()).unwrap();
    let appearance = &theme["theme"];
    assert_eq!(hex(&appearance["foreground"]), "#d0d0d0");
    assert_eq!(hex(&appearance["background"]), "#101418");
    assert_eq!(hex(&appearance["caret"]), "#58a6ff");
    assert_eq!((hex(&appearance["selection"]), appearance["selection"]["alpha"].as_f64()), ("#264f78".into(), Some(1.0)));
    assert!(theme["styles"].get("rtf").is_none());
    for family in ["text", "markdown", "code"] {
        let status = block(&theme, family, "Status bar");
        assert_eq!(hex(&status["character"]["foreground"]), "#c9d1d9");
        assert_eq!(hex(&status["block"]["background"]), "#30363d");
        let command = block(&theme, family, "Status bar command");
        assert_eq!(hex(&command["character"]["foreground"]), "#30363d");
        assert_eq!(hex(&command["block"]["background"]), "#c9d1d9");
        assert_eq!(hex(&block(&theme, family, "Builtin line number")["character"]["foreground"]), "#6e7681");
        let current = block(&theme, family, "Builtin current line number");
        assert_eq!((hex(&current["character"]["foreground"]), &current["character"]["bold"]), ("#e6edf3".into(), &Value::Bool(true)));
        let search = character(&theme, family, "* Incremental match").unwrap();
        assert_eq!(hex(&search["properties"]["background"]), "#e3b341");
        assert_eq!(hex(&search["properties"]["foreground"]), "#000000");
    }
    // Midnight's typography is retained.
    let midnight: Value = serde_json::from_slice(&crate::document::theme::default_json(0).unwrap()).unwrap();
    assert_eq!(block(&theme, "code", "Paragraph")["character"]["font_families"],
        block(&midnight, "code", "Paragraph")["character"]["font_families"]);
}

#[test]
fn code_styles_declare_only_differences_from_their_parents() {
    let (theme, _) = import(DARK);
    let properties = |id: &str| character(&theme, "code", id).map(|style| style["properties"].clone());
    let comment = properties("syntax:Comment").unwrap();
    assert_eq!((hex(&comment["foreground"]), &comment["slant"]), ("#7f8c8d".into(), &json!("Italic")));
    // Midnight's comment typography does not survive a scheme's own choice.
    assert!(comment.get("font_families").is_none());
    let statement = properties("syntax:Statement").unwrap();
    assert_eq!((hex(&statement["foreground"]), &statement["bold"]), ("#ff7b72".into(), &json!(true)));
    let keyword = properties("syntax:Keyword").unwrap();
    assert_eq!((hex(&keyword["foreground"]), &keyword["bold"]), ("#ffa657".into(), &json!(false)));
    // Conditional links to Statement and Code's Conditional inherits from it.
    assert!(properties("syntax:Conditional").is_none());
    // Midnight's own Keyword.import color is replaced by Vim's Include,
    // which links to PreProc like Code's Include style.
    assert!(properties("syntax:Include").is_none());
    let import = properties("syntax:Keyword.import").unwrap();
    assert_eq!(hex(&import["foreground"]), hex(&properties("syntax:PreProc").unwrap()["foreground"]));
    // Type.builtin has no Vim group and inherits Type again.
    assert!(properties("syntax:Type.builtin").is_none());
    let todo = properties("syntax:Todo").unwrap();
    assert_eq!((hex(&todo["foreground"]), hex(&todo["background"])), ("#ffffff".into(), "#bb0000".into()));
    // Title is a syntax link target without a built-in style.
    assert_eq!(hex(&properties("syntax:Title").unwrap()["foreground"]), "#d2a8ff");
    let sheet = code_style::parse_json(&serde_json::to_vec(&theme["styles"]["code"]).unwrap()).unwrap();
    let resolved = |name: &str| {
        let id = code_style::resolve_name(&sheet, name).unwrap().clone();
        sheet.named_character_declarations(Some(&id)).unwrap()
    };
    assert_eq!(resolved("Conditional").foreground, resolved("Statement").foreground);
    assert_eq!(resolved("Conditional").bold, Some(true));
}

#[test]
fn light_and_default_schemes_use_paper_without_document_colors() {
    // Without a Normal background, Vim's default 'background' is light.
    let (theme, base) = import("hi clear\nhi Comment guifg=#008800\nhi Title guifg=#aa00aa gui=bold\nhi Underlined guifg=#0000cc gui=underline\n");
    assert_eq!(base, "Paper");
    let paper: Value = serde_json::from_slice(PAPER).unwrap();
    assert_eq!(hex(&theme["theme"]["background"]), hex(&paper["theme"]["background"]));
    let (light, base) = import("hi Normal guifg=#202020 guibg=#fdf6e3\nset background=dark\n");
    assert_eq!((base, hex(&light["theme"]["background"])), ("Paper", "#fdf6e3".into()));
    let (dark, base) = import("set background=dark\nhi Comment guifg=#80a0ff\n");
    assert_eq!(base, "Midnight");
    let midnight: Value = serde_json::from_slice(MIDNIGHT).unwrap();
    assert_eq!(hex(&dark["theme"]["background"]), hex(&midnight["theme"]["background"]));
}

#[test]
fn document_styles_inherit_normal_colors_and_keep_typography() {
    // Paper colors headings, links, code and quotes; Midnight colors tables.
    for source in [DARK.replace("#101418", "#fdfdfd"), DARK.to_owned()] {
        let (theme, _) = import(&source);
        let preset: Value = serde_json::from_slice(if hex(&theme["theme"]["background"]) == "#fdfdfd" { PAPER } else { MIDNIGHT }).unwrap();
        let bytes = serde_json::to_vec(&theme["styles"]["markdown"]).unwrap();
        let (sheet, diagnostics) = StyleSheet::for_format(Format::Markdown).with_default_json(&bytes).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for id in ["Heading1", "Heading2", "Code Block", "Block quote", "Table cell", "Table header"] {
            let style = sheet
                .resolve_paragraph_style(&"Paragraph".into(), &id.into(), None, &Default::default(), &Default::default())
                .unwrap();
            assert!(style.character.foreground_is_default && style.character.background.is_none(), "{id}");
            assert!(style.background.is_none() && style.border_left_color.is_none() && style.border_top_color.is_none(), "{id}");
            // Typography and geometry are the preset's.
            let declared = block(&theme, "markdown", id);
            let original = block(&preset, "markdown", id);
            assert_eq!(declared["character"]["font_families"], original["character"]["font_families"], "{id}");
            assert_eq!(declared["block"]["border_left_width"], original["block"]["border_left_width"], "{id}");
        }
        for id in ["Link", "Code", "Comment", "Markdown reference"] {
            let properties = sheet.named_character_declarations(Some(&StyleId(id.into()))).unwrap();
            assert!(properties.foreground.is_none() && properties.background.is_none(), "{id}");
        }
        assert_eq!(character(&theme, "markdown", "Link").unwrap()["properties"]["underline"], json!(true));
        // Chrome keeps the scheme's colors.
        assert_eq!(hex(&block(&theme, "markdown", "Status bar")["block"]["background"]), "#30363d");
        let search = character(&theme, "markdown", "* Incremental match").unwrap();
        assert_eq!(hex(&search["properties"]["background"]), "#e3b341");
    }
}

#[test]
fn reverse_status_cursor_and_visual_fall_back_to_normal_colors() {
    let (theme, _) = import(
        "hi Normal guifg=#ffffff guibg=#000000\nhi StatusLine guifg=#444444 guibg=#eeeeee gui=reverse,bold\n\
         hi Visual guifg=#666666 guibg=#ffffff\nhi clear Cursor\nhi CursorLineNr guifg=#ff0000\nhi clear CursorLineNr\n",
    );
    let status = block(&theme, "text", "Status bar");
    assert_eq!(hex(&status["character"]["foreground"]), "#eeeeee");
    assert_eq!(hex(&status["block"]["background"]), "#444444");
    // Vim's Visual text color is not drawn, so a selection that would hide
    // Normal text is translucent.
    assert_eq!(theme["theme"]["selection"]["alpha"].as_f64(), Some(0.4f32 as f64));
    // A cleared cursor is drawn with the text color.
    assert_eq!(hex(&theme["theme"]["caret"]), "#ffffff");
    // A cleared CursorLineNr shows like the other numbers.
    let current = block(&theme, "text", "Builtin current line number");
    assert!(current["character"].get("foreground").is_none_or(Value::is_null));
}

#[test]
fn every_imported_style_family_loads_without_diagnostics() {
    let (theme, _) = import(DARK);
    for (family, format) in [("text", Format::PlainText), ("markdown", Format::Markdown)] {
        let bytes = serde_json::to_vec(&theme["styles"][family]).unwrap();
        let (_, diagnostics) = StyleSheet::for_format(format).with_default_json(&bytes).unwrap();
        assert!(diagnostics.is_empty(), "{family}: {diagnostics:?}");
    }
}

#[test]
fn files_without_highlights_are_rejected() {
    assert!(import_vim_colorscheme("empty.vim", b"\" just a comment\nlet x = 1\n").is_err());
    assert!(import_vim_colorscheme("binary.vim", &[0xff, 0xfe, 0x00, b'h']).is_err());
    assert!(import_vim_colorscheme("huge.vim", &vec![b' '; MAX_VIM_COLORSCHEME_BYTES + 1]).is_err());
}

#[test]
fn imported_names_are_portable_and_numbered_on_collision() {
    assert_eq!(unique_theme_name("desert", &["Paper", "Midnight"]), "desert");
    assert_eq!(unique_theme_name("desert", &["Desert"]), "desert (2)");
    assert_eq!(unique_theme_name("desert", &["desert", "desert (2)", "DESERT (3)"]), "desert (4)");
    // Default is reserved.
    assert_eq!(unique_theme_name("default", &[]), "default (2)");
    assert_eq!(unique_theme_name("a/b:c", &[]), "a_b_c");
    let long = "x".repeat(40);
    assert_eq!(unique_theme_name(&long, &[]), "x".repeat(32));
    assert_eq!(unique_theme_name(&long, &["x".repeat(32).as_str()]), format!("{} (2)", "x".repeat(28)));
    assert_eq!(unique_theme_name("", &[]), "Vim theme");
    assert_eq!(unique_theme_name("con", &[]), "con (2)");
}

/// Every scheme in `VIEM_VIM_COLORS` (for example `/usr/share/vim/vim91/colors`)
/// imports into a valid theme.
#[test]
fn installed_vim_color_schemes_import() {
    let Ok(directory) = std::env::var("VIEM_VIM_COLORS") else { return };
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "vim") {
            continue;
        }
        let imported = import_vim_colorscheme(&path.display().to_string(), &std::fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        validate_json(&imported.theme).unwrap();
        println!("{} → {} ({} warnings)", path.display(), imported.base, imported.warnings.len());
        for warning in &imported.warnings {
            println!("  {}:{}: {}", warning.file, warning.line, warning.message);
        }
    }
}
