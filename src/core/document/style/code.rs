//! The independent application-wide Code style authority. No buffer history
//! or source artifact participates in a settings transaction.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock, RwLock};

static GLOBAL: OnceLock<RwLock<Arc<StyleSheet>>> = OnceLock::new();
fn authority() -> &'static RwLock<Arc<StyleSheet>> {
    GLOBAL.get_or_init(|| RwLock::new(Arc::new(default_sheet())))
}
pub fn snapshot() -> Arc<StyleSheet> {
    authority()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn default_sheet() -> StyleSheet {
    let mut sheet = StyleSheet::default();
    sheet
        .block_styles
        .retain(|id, _| *id == sheet.base_document || *id == sheet.base_paragraph);
    sheet
        .block_metadata
        .retain(|id, _| sheet.block_styles.contains_key(id));
    sheet
        .character_styles
        .retain(|id, _| *id == sheet.base_character);
    sheet
        .character_metadata
        .retain(|id, _| sheet.character_styles.contains_key(id));
    sheet
        .block_styles
        .get_mut(&sheet.base_document)
        .unwrap()
        .character
        .font_families = Some(vec!["monospace".into()]);
    let families: &[(&str, &[&str], (u8, u8, u8))] = &[
        (
            "keyword",
            &[
                "@keyword",
                "@keyword.function",
                "@keyword.return",
                "@keyword.operator",
                "@keyword.import",
                "@keyword.coroutine",
                "@include",
                "@preproc",
                "@label",
                "@conditional",
                "@repeat",
                "@exception",
                "Statement",
                "Conditional",
                "Repeat",
                "Label",
                "Keyword",
                "Exception",
                "PreProc",
                "Include",
                "Define",
                "Macro",
                "PreCondit",
            ],
            (170, 65, 153),
        ),
        (
            "string",
            &[
                "@string",
                "@string.special",
                "@string.escape",
                "@string.regex",
                "@escape",
                "@text.uri",
                "@character",
                "@character.special",
                "String",
                "Character",
                "SpecialChar",
            ],
            (38, 132, 77),
        ),
        (
            "comment",
            &["@comment", "@comment.documentation", "Comment", "Todo"],
            (115, 123, 130),
        ),
        (
            "function",
            &[
                "@function",
                "@function.builtin",
                "@function.call",
                "@function.method",
                "@function.macro",
                "@function.macro.builtin",
                "@function.special",
                "@method",
                "@method.call",
                "@constructor",
                "Function",
            ],
            (45, 110, 180),
        ),
        (
            "type",
            &[
                "@type",
                "@type.builtin",
                "@type.definition",
                "@type.qualifier",
                "@storageclass",
                "@module",
                "@namespace",
                "Type",
                "StorageClass",
                "Structure",
                "Typedef",
            ],
            (154, 102, 42),
        ),
        (
            "constant",
            &[
                "@constant",
                "@constant.builtin",
                "@constant.macro",
                "@number",
                "@number.float",
                "@float",
                "@boolean",
                "Constant",
                "Number",
                "Boolean",
                "Float",
            ],
            (155, 95, 190),
        ),
        (
            "special",
            &[
                "@operator",
                "@punctuation.delimiter",
                "@punctuation.bracket",
                "@punctuation.special",
                "@delimiter",
                "@tag",
                "@tag.attribute",
                "@attribute",
                "Operator",
                "Special",
                "SpecialComment",
                "Delimiter",
                "Debug",
                "Error",
            ],
            (75, 125, 145),
        ),
        (
            "identifier",
            &[
                "@variable",
                "@variable.parameter",
                "@variable.builtin",
                "@property",
                "@property.definition",
                "@field",
                "@parameter",
                "@parameter.builtin",
                "Identifier",
            ],
            (80, 100, 140),
        ),
    ];
    for (_, names, (red, green, blue)) in families {
        for name in *names {
            let id = StyleId(format!("syntax:{name}"));
            sheet.character_styles.insert(
                id.clone(),
                CharacterStyle {
                    id: id.clone(),
                    based_on: Some(sheet.base_character.clone()),
                    properties: CharacterProperties {
                        foreground: Some(Color {
                            red: *red as f32 / 255.,
                            green: *green as f32 / 255.,
                            blue: *blue as f32 / 255.,
                            alpha: 1.,
                        }),
                        ..Default::default()
                    },
                },
            );
            sheet
                .character_metadata
                .insert(id, StyleDefinitionMetadata::generated(*name));
        }
    }
    for name in ["@embedded", "@spell"] {
        let id = StyleId(format!("syntax:{name}"));
        sheet.character_styles.insert(
            id.clone(),
            CharacterStyle {
                id: id.clone(),
                based_on: Some(sheet.base_character.clone()),
                properties: Default::default(),
            },
        );
        sheet
            .character_metadata
            .insert(id, StyleDefinitionMetadata::generated(name));
    }
    sheet
}

pub fn resolve_name<'a>(sheet: &'a StyleSheet, name: &str) -> Option<&'a StyleId> {
    sheet.character_styles.keys().find(|id| {
        sheet
            .character_metadata
            .get(*id)
            .is_some_and(|m| m.display_name == name)
    })
}

#[derive(Serialize, Deserialize)]
struct BlockEntry {
    name: String,
    #[serde(flatten)]
    style: BlockStyle,
}
#[derive(Serialize, Deserialize)]
struct CharacterEntry {
    name: String,
    #[serde(flatten)]
    style: CharacterStyle,
}
#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    block_styles: Vec<BlockEntry>,
    #[serde(default)]
    character_styles: Vec<CharacterEntry>,
    #[serde(default)]
    suppressed_character_ids: BTreeSet<StyleId>,
}

fn validate(sheet: &StyleSheet) -> Result<(), String> {
    sheet
        .validate_block_cycles()
        .map_err(|e| format!("{e:?}"))?;
    sheet
        .validate_character_cycles()
        .map_err(|e| format!("{e:?}"))?;
    let mut names = BTreeSet::new();
    for (id, style) in &sheet.block_styles {
        validate_character_properties(id, &style.character).map_err(|e| format!("{e:?}"))?;
        validate_block_properties(style).map_err(|e| format!("{e:?}"))?;
        validate_definition_metadata(id, &sheet.block_metadata[id])
            .map_err(|e| format!("{e:?}"))?;
        if style.next_paragraph_style.is_some() {
            return Err("Code paragraphs always use Base Paragraph".into());
        }
    }
    for (id, metadata) in &sheet.character_metadata {
        if metadata.display_name.trim().is_empty() || !names.insert(&metadata.display_name) {
            return Err("Code style names must be nonempty and unique".into());
        }
        validate_definition_metadata(id, metadata).map_err(|e| format!("{e:?}"))?;
        validate_character_properties(id, &sheet.character_styles[id].properties)
            .map_err(|e| format!("{e:?}"))?;
    }
    if sheet.block_styles.len() != 2
        || !sheet.block_styles.contains_key(&sheet.base_document)
        || !sheet.block_styles.contains_key(&sheet.base_paragraph)
        || !sheet.character_styles.contains_key(&sheet.base_character)
    {
        return Err("Code base styles are required".into());
    }
    if sheet.block_styles[&sheet.base_document].role != BlockRole::Document
        || sheet.block_styles[&sheet.base_document].based_on.is_some()
        || sheet.block_styles[&sheet.base_paragraph].role != BlockRole::Paragraph
        || sheet.block_styles[&sheet.base_paragraph].based_on.as_ref() != Some(&sheet.base_document)
        || sheet.character_styles[&sheet.base_character]
            .based_on
            .is_some()
    {
        return Err("Invalid Code base relationships".into());
    }
    Ok(())
}

pub fn export_json() -> Result<Vec<u8>, String> {
    export_snapshot(&snapshot())
}
pub fn export_snapshot(sheet: &StyleSheet) -> Result<Vec<u8>, String> {
    let defaults = default_sheet();
    let suppressed_character_ids = defaults
        .character_styles
        .keys()
        .filter(|id| !sheet.character_styles.contains_key(*id))
        .cloned()
        .collect();
    let file = File {
        version: 1,
        block_styles: sheet
            .block_styles
            .values()
            .filter(|s| {
                defaults.block_styles.get(&s.id) != Some(s)
                    || defaults.block_metadata.get(&s.id) != sheet.block_metadata.get(&s.id)
            })
            .map(|s| BlockEntry {
                name: sheet.block_metadata[&s.id].display_name.clone(),
                style: s.clone(),
            })
            .collect(),
        character_styles: sheet
            .character_styles
            .values()
            .filter(|s| {
                defaults.character_styles.get(&s.id) != Some(s)
                    || defaults.character_metadata.get(&s.id) != sheet.character_metadata.get(&s.id)
            })
            .map(|s| CharacterEntry {
                name: sheet.character_metadata[&s.id].display_name.clone(),
                style: s.clone(),
            })
            .collect(),
        suppressed_character_ids,
    };
    serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())
}

pub fn parse_json(bytes: &[u8]) -> Result<StyleSheet, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Code stylesheet exceeds 4 MiB".into());
    }
    let mut sheet = default_sheet();
    if !bytes.is_empty() {
        let file: File = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if file.version != 1 || file.character_styles.len() > 4096 {
            return Err("Unsupported Code stylesheet".into());
        }
        for id in file.suppressed_character_ids {
            sheet.character_styles.remove(&id);
            sheet.character_metadata.remove(&id);
        }
        let mut seen = BTreeSet::new();
        for entry in file.block_styles {
            if !seen.insert((0, entry.style.id.clone())) {
                return Err("Duplicate style ID".into());
            }
            sheet.block_metadata.insert(
                entry.style.id.clone(),
                StyleDefinitionMetadata::generated(entry.name),
            );
            sheet
                .block_styles
                .insert(entry.style.id.clone(), entry.style);
        }
        for entry in file.character_styles {
            if !seen.insert((1, entry.style.id.clone())) {
                return Err("Duplicate style ID".into());
            }
            sheet.character_metadata.insert(
                entry.style.id.clone(),
                StyleDefinitionMetadata::generated(entry.name),
            );
            sheet
                .character_styles
                .insert(entry.style.id.clone(), entry.style);
        }
    }
    validate(&sheet)?;
    Ok(sheet)
}

pub fn replace_json(bytes: &[u8]) -> Result<Arc<StyleSheet>, String> {
    let mut sheet = parse_json(bytes)?;
    let mut guard = authority().write().unwrap_or_else(|e| e.into_inner());
    sheet.revision = StyleSheetRevision(
        guard
            .revision
            .0
            .checked_add(1)
            .ok_or("Style generation exhausted")?,
    );
    *guard = Arc::new(sheet);
    Ok(guard.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        syntax::{SyntaxRun, SyntaxStyleName},
        Document, Encoding, Format,
    };
    use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

    #[test]
    fn defaults_cover_every_bundled_highlight_capture_and_reject_non_code_paragraph_semantics() {
        let sheet = default_sheet();
        for language in [
            "c",
            "cpp",
            "rust",
            "swift",
            "objc",
            "c_sharp",
            "javascript",
            "typescript",
            "tsx",
            "python",
        ] {
            let package =
                crate::document::syntax::treesitter::TreeSitterPackage::bundled(language).unwrap();
            for capture in package.highlight_capture_names() {
                assert!(
                    resolve_name(&sheet, &format!("@{capture}")).is_some(),
                    "missing {language}: @{capture}"
                );
            }
        }
        let mut invalid = sheet.clone();
        invalid
            .block_styles
            .get_mut(&invalid.base_paragraph)
            .unwrap()
            .next_paragraph_style = Some(invalid.base_paragraph.clone());
        assert!(parse_json(&export_snapshot(&invalid).unwrap()).is_err());
        let mut invalid = sheet;
        invalid
            .block_styles
            .get_mut(&invalid.base_document)
            .unwrap()
            .block
            .line_spacing = Some(LineSpacing::Normal);
        assert!(parse_json(&export_snapshot(&invalid).unwrap()).is_err());
    }

    #[test]
    fn persisted_names_suppression_and_validation_are_exact() {
        let mut sheet = default_sheet();
        let id = resolve_name(&sheet, "@keyword").unwrap().clone();
        sheet.character_metadata.get_mut(&id).unwrap().display_name = "Custom keyword".into();
        let removed = resolve_name(&sheet, "Comment").unwrap().clone();
        sheet.remove_character_style(&removed, false).unwrap();
        let encoded = export_snapshot(&sheet).unwrap();
        let restored = parse_json(&encoded).unwrap();
        assert!(resolve_name(&restored, "@keyword").is_none());
        assert!(resolve_name(&restored, "Comment").is_none());
        assert_eq!(resolve_name(&restored, "Custom keyword"), Some(&id));
        assert!(resolve_name(&restored, "custom keyword").is_none());
        let mut duplicate = serde_json::from_slice::<serde_json::Value>(&encoded).unwrap();
        duplicate["character_styles"][0]["name"] = serde_json::Value::String("String".into());
        assert!(parse_json(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    }

    #[test]
    fn named_runs_color_and_metrics_have_separate_layout_effects() {
        let mut document =
            Document::from_bytes(b"let answer = 42;".to_vec(), Encoding::Utf8, Format::Code)
                .unwrap();
        let source = document.source_bytes();
        let revision = document.revision();
        let undo = document.history_status().node_count;
        let run = SyntaxRun {
            range: 0..3,
            name: SyntaxStyleName("@keyword".into()),
            origin: "rust:@keyword".into(),
            priority: 100,
        };
        let mut sheet = default_sheet();
        let id = resolve_name(&sheet, "@keyword").unwrap().clone();
        document.install_code_presentation(Arc::new(sheet.clone()), &[run.clone()]);
        let mut view = ViewLayout::new(400., 200.);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        engine.relayout(&document, &mut view).unwrap();
        let calls = engine.provider().request_calls();
        sheet
            .character_styles
            .get_mut(&id)
            .unwrap()
            .properties
            .foreground = Some(Color {
            red: 1.,
            green: 0.,
            blue: 0.,
            alpha: 1.,
        });
        sheet.revision.0 += 1;
        document.install_code_presentation(Arc::new(sheet.clone()), &[run.clone()]);
        view.invalidate_syntax_presentation(false);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(
            engine.provider().request_calls(),
            calls,
            "color-only syntax changes must reuse shaping"
        );
        sheet.character_styles.get_mut(&id).unwrap().properties.size = Some(24.);
        sheet.revision.0 += 1;
        document.install_code_presentation(Arc::new(sheet.clone()), &[run]);
        view.invalidate_syntax_presentation(true);
        engine.relayout(&document, &mut view).unwrap();
        assert!(engine.provider().request_calls() > calls);
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status().node_count, undo);
        assert!(!document.is_dirty());
        document.install_code_presentation(Arc::new(sheet), &[]);
        assert!(
            document.projection().style_spans().is_empty(),
            "completed empty coverage clears earlier runs"
        );
    }
}

pub fn edit(
    expected: StyleSheetRevision,
    edit: StyleDefinitionEdit,
) -> Result<Arc<StyleSheet>, String> {
    let mut guard = authority().write().unwrap_or_else(|e| e.into_inner());
    if expected != guard.revision {
        return Err("Stale Code stylesheet".into());
    }
    let mut next = guard.as_ref().clone();
    let revision = StyleSheetRevision(
        expected
            .0
            .checked_add(1)
            .ok_or("Style generation exhausted")?,
    );
    if let StyleDefinitionEdit::DeleteCharacter(id) = &edit {
        if *id != next.base_character {
            let parent = next
                .character_styles
                .get(id)
                .and_then(|s| s.based_on.clone())
                .unwrap_or_else(|| next.base_character.clone());
            for child in next
                .character_styles
                .values_mut()
                .filter(|s| s.based_on.as_ref() == Some(id))
            {
                child.based_on = Some(parent.clone());
            }
        }
    }
    next.apply_configuration_edit(&edit, revision, false)
        .map_err(|e| format!("{e:?}"))?;
    validate(&next)?;
    *guard = Arc::new(next);
    Ok(guard.clone())
}
