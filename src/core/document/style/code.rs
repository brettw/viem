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
        .retain(|id, _| *id == sheet.base_paragraph);
    sheet
        .block_metadata
        .retain(|id, _| sheet.block_styles.contains_key(id));
    sheet
        .character_styles
        .retain(|id, _| id.is_internal());
    sheet
        .character_metadata
        .retain(|id, _| sheet.character_styles.contains_key(id));
    sheet
        .block_styles
        .get_mut(&sheet.base_paragraph)
        .unwrap()
        .character
        .font_families = Some(vec!["monospace".into()]);
    let families: &[(&str, &[&str], (u8, u8, u8))] = &[
        (
            "Statement",
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
            "String",
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
            "Comment",
            &["@comment", "@comment.documentation", "Comment", "Todo"],
            (115, 123, 130),
        ),
        (
            "Function",
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
            "Type",
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
            "Constant",
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
            "Special",
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
            "Identifier",
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
    for (root, names, (red, green, blue)) in families {
        // Canonical captures and Vim groups share one definition. Add missing
        // intermediate capture prefixes so dotted inheritance is always direct.
        let mut canonical = BTreeSet::new();
        for name in *names {
            let name = canonical_capture_name(name);
            let mut prefix = name.as_str();
            canonical.insert(name.clone());
            while let Some((parent, _)) = prefix.rsplit_once('.') {
                canonical.insert(parent.to_owned());
                prefix = parent;
            }
        }
        for name in canonical {
            let id = StyleId(format!("syntax:{name}"));
            let parent = (name != *root)
                .then(|| StyleId(format!("syntax:{}", default_parent(&name, root))));
            sheet.character_styles.insert(
                id.clone(),
                CharacterStyle {
                    id: id.clone(),
                    based_on: parent,
                    properties: CharacterProperties {
                        foreground: (name == *root).then_some(Color {
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
                .insert(id, StyleDefinitionMetadata::generated(name));
        }
    }
    // `@none` is Neovim's explicit reset: it wins over an enclosing capture,
    // such as a string around a C# interpolation, and shows default text.
    let empty: &[&str] = &["Embedded", "Spell", "None"];
    for &name in empty {
        let id = StyleId(format!("syntax:{name}"));
        sheet.character_styles.insert(
            id.clone(),
            CharacterStyle {
                id: id.clone(),
                based_on: None,
                properties: Default::default(),
            },
        );
        sheet
            .character_metadata
            .insert(id, StyleDefinitionMetadata::generated(name));
    }
    // Neovim's markup captures, used by the JSX queries for text inside
    // elements such as `<h1>` or `<em>`. They carry typographic meaning
    // rather than a color.
    let bold = CharacterProperties { bold: Some(true), ..Default::default() };
    let markup: [(&str, CharacterProperties); 16] = [
        ("Markup", Default::default()),
        ("Markup.heading", bold.clone()),
        ("Markup.heading.1", Default::default()),
        ("Markup.heading.2", Default::default()),
        ("Markup.heading.3", Default::default()),
        ("Markup.heading.4", Default::default()),
        ("Markup.heading.5", Default::default()),
        ("Markup.heading.6", Default::default()),
        ("Markup.strong", bold),
        ("Markup.italic", CharacterProperties { slant: Some(FontSlant::Italic), ..Default::default() }),
        ("Markup.underline", CharacterProperties { underline: Some(true), ..Default::default() }),
        ("Markup.strikethrough", CharacterProperties { strikethrough: Some(true), ..Default::default() }),
        ("Markup.raw", Default::default()),
        ("Markup.link", Default::default()),
        ("Markup.link.label", Default::default()),
        ("Markup.link.url", CharacterProperties { underline: Some(true), ..Default::default() }),
    ];
    for (name, properties) in markup {
        let id = StyleId(format!("syntax:{name}"));
        let based_on = name.rsplit_once('.').map(|(parent, _)| StyleId(format!("syntax:{parent}")));
        sheet.character_styles.insert(id.clone(), CharacterStyle { id: id.clone(), based_on, properties });
        sheet.character_metadata.insert(id, StyleDefinitionMetadata::generated(name));
    }
    sheet
}

/// Provider normalization, not fuzzy style lookup. Preserve everything after
/// the first character, including the spelling of dotted capture segments.
pub(crate) fn canonical_capture_name(name: &str) -> String {
    let name = name.strip_prefix('@').unwrap_or(name);
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect()
}

/// Explicit default relationships. Every dotted definition has its immediate
/// lexical parent; unknown provider names still require an exact definition.
fn default_parent<'a>(name: &'a str, root: &'a str) -> &'a str {
    if let Some((parent, _)) = name.rsplit_once('.') {
        return parent;
    }
    match name {
        "Preproc" => "PreProc",
        "Include" | "Define" | "Macro" | "PreCondit" => "PreProc",
        "Escape" => "SpecialChar",
        "Storageclass" => "StorageClass",
        "Float" => "Number",
        "Punctuation" => "Delimiter",
        _ => root,
    }
}

/// At most this many implicit definitions exist at once.
pub const MAX_IMPLICIT_DEFINITIONS: usize = 1024;
/// Distinct from built-in `syntax:` IDs, so a regenerated name never revives a
/// deleted built-in definition whose suppression is persisted.
const IMPLICIT_ID_PREFIX: &str = "implicit:";

/// A syntax name's appearance: its own definition, else its nearest defined
/// dotted ancestor. This equals the appearance of the implicit definitions
/// that will be generated for it, so rendering never waits for generation.
pub fn resolve_syntax_name<'a>(sheet: &'a StyleSheet, name: &str) -> Option<&'a StyleId> {
    let mut name = name;
    loop {
        if let Some(id) = resolve_name(sheet, name) {
            return Some(id);
        }
        name = name.rsplit_once('.')?.0;
    }
}

#[derive(Debug, Default)]
pub struct Materialized {
    /// The published sheet when any definition was generated.
    pub sheet: Option<Arc<StyleSheet>>,
    /// Some names were left to the ancestor walk by `MAX_IMPLICIT_DEFINITIONS`.
    pub limited: bool,
}

/// Generate an empty implicit definition for each name without one, and for
/// its missing dotted ancestors. A dotted name is based on its dot-parent; an
/// undotted name is a root. This is one serialized global-stylesheet operation
/// that adds no document history and changes no appearance.
pub fn materialize<'a>(names: impl IntoIterator<Item = &'a str> + Clone) -> Materialized {
    if missing_names(&snapshot(), names.clone()).is_empty() {
        return Materialized::default();
    }
    let mut guard = authority().write().unwrap_or_else(|e| e.into_inner());
    let Some(revision) = guard.revision.0.checked_add(1) else {
        return Materialized::default();
    };
    let mut next = guard.as_ref().clone();
    let limited = generate_implicit(&mut next, names);
    let mut result = Materialized { sheet: None, limited };
    if next.implicit_characters == guard.implicit_characters {
        return result;
    }
    next.revision = StyleSheetRevision(revision);
    debug_assert_eq!(validate(&next), Ok(()));
    *guard = Arc::new(next);
    result.sheet = Some(guard.clone());
    result
}

/// Names, and their dotted ancestors, that have no definition.
fn missing_names<'a>(sheet: &StyleSheet, names: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let defined = sheet
        .character_metadata
        .values()
        .map(|metadata| metadata.display_name.as_str())
        .collect::<BTreeSet<_>>();
    let mut missing = BTreeSet::new();
    for name in names {
        let mut name = name;
        while !name.trim().is_empty() && !defined.contains(name) {
            missing.insert(name.to_owned());
            match name.rsplit_once('.') {
                Some((parent, _)) => name = parent,
                None => break,
            }
        }
    }
    missing
}

/// Returns whether `MAX_IMPLICIT_DEFINITIONS` left any name undefined.
fn generate_implicit<'a>(sheet: &mut StyleSheet, names: impl IntoIterator<Item = &'a str>) -> bool {
    // A name's dotted prefixes are generated first so it can link to them.
    let mut order = missing_names(sheet, names).into_iter().collect::<Vec<_>>();
    order.sort_by_key(|name| name.matches('.').count());
    for name in order {
        if sheet.implicit_characters.len() >= MAX_IMPLICIT_DEFINITIONS {
            return true;
        }
        let based_on = match name.rsplit_once('.') {
            Some((parent, _)) => match resolve_name(sheet, parent) {
                Some(id) => Some(id.clone()),
                None => continue,
            },
            None => None,
        };
        let mut id = StyleId(format!("{IMPLICIT_ID_PREFIX}{name}"));
        let mut suffix = 2;
        while sheet.character_styles.contains_key(&id) {
            id = StyleId(format!("{IMPLICIT_ID_PREFIX}{name}#{suffix}"));
            suffix += 1;
        }
        sheet.character_styles.insert(
            id.clone(),
            CharacterStyle {
                id: id.clone(),
                based_on,
                properties: Default::default(),
            },
        );
        sheet
            .character_metadata
            .insert(id.clone(), StyleDefinitionMetadata::generated(name));
        sheet.deleted_configuration_characters.remove(&id);
        sheet.implicit_characters.insert(id);
    }
    false
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
    let internal = StyleId::incremental_match();
    if !sheet.character_styles.get(&internal).is_some_and(|style| style.based_on.is_none())
        || !sheet.character_metadata.get(&internal).is_some_and(|metadata| metadata.display_name == "Incremental match")
    {
        return Err("The internal Incremental match style is required and cannot be renamed or reparented".into());
    }
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
    if sheet.block_styles.len() != 1 || !sheet.block_styles.contains_key(&sheet.base_paragraph) {
        return Err("Code Base Paragraph is required".into());
    }
    if matches!(sheet.block_styles[&sheet.base_paragraph].character.size, Some(FontSize::Percentage(_))) {
        return Err("Code Base Paragraph font size must use points".into());
    }
    if sheet.block_styles[&sheet.base_paragraph].role != BlockRole::Paragraph
        || sheet.block_styles[&sheet.base_paragraph].based_on.is_some() {
        return Err("Invalid Code base relationships".into());
    }
    Ok(())
}

pub fn export_json() -> Result<Vec<u8>, String> {
    export_snapshot(&snapshot())
}
pub fn export_snapshot(sheet: &StyleSheet) -> Result<Vec<u8>, String> {
    let defaults = default_sheet();
    let persisted = |s: &&CharacterStyle| {
        !sheet.implicit_characters.contains(&s.id)
            && (defaults.character_styles.get(&s.id) != Some(*s)
                || defaults.character_metadata.get(&s.id) != sheet.character_metadata.get(&s.id))
    };
    // An edited definition may be based on an implicit one. Persist that
    // ancestry too, so the saved sheet never names a missing parent.
    let mut required = BTreeSet::new();
    for style in sheet.character_styles.values().filter(persisted) {
        let mut parent = style.based_on.as_ref();
        while let Some(id) = parent.filter(|id| sheet.implicit_characters.contains(*id)) {
            if !required.insert(id.clone()) {
                break;
            }
            parent = sheet.character_styles.get(id).and_then(|s| s.based_on.as_ref());
        }
    }
    let suppressed_character_ids = defaults
        .character_styles
        .keys()
        .filter(|id| !sheet.character_styles.contains_key(*id))
        .cloned()
        .collect();
    let file = File {
        version: 3,
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
            .filter(|s| persisted(s) || required.contains(&s.id))
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
        if file.version != 3 || file.character_styles.len() > 4096 {
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
    // Implicit definitions are not in the file. Carry the current ones over,
    // with the same IDs, so a reload does not drop names that are still in
    // use or the style an open editor has selected.
    let implicit = guard
        .implicit_characters
        .iter()
        .filter_map(|id| guard.character_metadata.get(id))
        .map(|metadata| metadata.display_name.clone())
        .collect::<Vec<_>>();
    generate_implicit(&mut sheet, implicit.iter().map(String::as_str));
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
mod linking_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        syntax::{SyntaxRun, SyntaxStyleName},
        Document, Encoding, Format,
    };
    use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

    fn named<'a>(sheet: &'a StyleSheet, name: &str) -> &'a CharacterStyle {
        &sheet.character_styles[resolve_name(sheet, name).unwrap()]
    }

    #[test]
    fn implicit_definitions_link_dotted_ancestry_and_resolve_like_their_parent() {
        let mut sheet = default_sheet();
        assert_eq!(
            resolve_syntax_name(&sheet, "Keyword.directive.define"),
            resolve_name(&sheet, "Keyword"),
            "an ungenerated name already has its parent's appearance"
        );
        assert!(resolve_syntax_name(&sheet, "Unknown.root").is_none());
        assert!(!generate_implicit(
            &mut sheet,
            ["Keyword.directive.define", "Keyword.directive", "Unknown.root", "Keyword"]
        ));
        let keyword = resolve_name(&sheet, "Keyword").unwrap().clone();
        let directive = named(&sheet, "Keyword.directive");
        let define = named(&sheet, "Keyword.directive.define");
        let unknown = named(&sheet, "Unknown");
        assert_eq!(directive.based_on.as_ref(), Some(&keyword));
        assert_eq!(define.based_on.as_ref(), Some(&directive.id));
        assert_eq!(unknown.based_on, None, "an undotted name is a root");
        assert_eq!(named(&sheet, "Unknown.root").based_on.as_ref(), Some(&unknown.id));
        for style in [directive, define, unknown] {
            assert!(sheet.is_implicit_character(&style.id));
            assert_eq!(style.properties, CharacterProperties::default());
            assert!(style.id.0.starts_with(IMPLICIT_ID_PREFIX));
        }
        assert!(!sheet.is_implicit_character(&keyword), "existing definitions are untouched");
        assert_eq!(validate(&sheet), Ok(()));
        let before = sheet.clone();
        assert!(!generate_implicit(&mut sheet, ["Keyword.directive.define"]));
        assert_eq!(sheet, before, "generation is idempotent");
    }

    #[test]
    fn implicit_definitions_persist_only_after_an_edit_or_as_a_needed_parent() {
        let mut sheet = default_sheet();
        generate_implicit(&mut sheet, ["Keyword.directive.define", "Variable.member"]);
        let exported = String::from_utf8(export_snapshot(&sheet).unwrap()).unwrap();
        assert!(!exported.contains(IMPLICIT_ID_PREFIX), "{exported}");
        // Editing the leaf persists it and the implicit parent it names.
        let mut define = named(&sheet, "Keyword.directive.define").clone();
        define.properties.bold = Some(true);
        let id = define.id.clone();
        sheet.apply_configuration_edit(
            &StyleDefinitionEdit::UpdateCharacter(define),
            StyleSheetRevision(sheet.revision.0 + 1),
            false,
        )
        .unwrap();
        sheet.implicit_characters.remove(&id);
        let reloaded = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
        assert_eq!(named(&reloaded, "Keyword.directive.define").properties.bold, Some(true));
        assert!(resolve_name(&reloaded, "Keyword.directive").is_some());
        assert!(resolve_name(&reloaded, "Variable.member").is_none());
        assert!(reloaded.implicit_characters.is_empty());
        // A saved definition (including its declaration-free parent) remains
        // ordinary through a later save and cold load without syntax discovery.
        let cold = parse_json(&export_snapshot(&reloaded).unwrap()).unwrap();
        assert_eq!(named(&cold, "Keyword.directive.define"), named(&reloaded, "Keyword.directive.define"));
        assert_eq!(named(&cold, "Keyword.directive"), named(&reloaded, "Keyword.directive"));
        assert!(cold.implicit_characters.is_empty());
    }

    #[test]
    fn deleted_and_renamed_definitions_are_regenerated_under_a_distinct_id() {
        let mut sheet = default_sheet();
        let comment = resolve_name(&sheet, "Comment").unwrap().clone();
        sheet.character_styles.remove(&comment);
        sheet.character_metadata.remove(&comment);
        generate_implicit(&mut sheet, ["Comment"]);
        let regenerated = named(&sheet, "Comment");
        assert_ne!(regenerated.id, comment, "a deleted built-in stays suppressed");
        assert_eq!(regenerated.properties, CharacterProperties::default());
        let exported = String::from_utf8(export_snapshot(&sheet).unwrap()).unwrap();
        assert!(exported.contains(&comment.0), "suppression is still persisted: {exported}");
        // Renaming ends the association; the name gets a new definition.
        let first = regenerated.id.clone();
        sheet.character_metadata.insert(first.clone(), StyleDefinitionMetadata::generated("Renamed"));
        sheet.implicit_characters.remove(&first);
        generate_implicit(&mut sheet, ["Comment"]);
        let second = named(&sheet, "Comment").id.clone();
        assert_ne!(second, first);
        assert_eq!(resolve_name(&sheet, "Renamed"), Some(&first));
    }

    #[test]
    fn replacing_the_file_keeps_implicit_definitions_and_their_ids() {
        let mut current = default_sheet();
        generate_implicit(&mut current, ["Keyword.directive"]);
        let id = resolve_name(&current, "Keyword.directive").unwrap().clone();
        let names = current
            .implicit_characters
            .iter()
            .map(|id| current.character_metadata[id].display_name.clone())
            .collect::<Vec<_>>();
        let mut reloaded = parse_json(&export_snapshot(&current).unwrap()).unwrap();
        assert!(resolve_name(&reloaded, "Keyword.directive").is_none());
        generate_implicit(&mut reloaded, names.iter().map(String::as_str));
        assert_eq!(resolve_name(&reloaded, "Keyword.directive"), Some(&id));
        assert!(reloaded.is_implicit_character(&id));
    }

    #[test]
    fn implicit_definitions_are_limited() {
        let mut sheet = default_sheet();
        let names = (0..MAX_IMPLICIT_DEFINITIONS + 5)
            .map(|n| format!("Generated{n}"))
            .collect::<Vec<_>>();
        assert!(generate_implicit(&mut sheet, names.iter().map(String::as_str)));
        assert_eq!(sheet.implicit_characters.len(), MAX_IMPLICIT_DEFINITIONS);
        assert_eq!(validate(&sheet), Ok(()));
    }

    #[test]
    fn every_bundled_highlight_capture_has_an_appearance_and_non_code_paragraph_semantics_are_rejected() {
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
            "json",
        ] {
            let package =
                crate::document::syntax::treesitter::TreeSitterPackage::bundled(language).unwrap();
            for capture in package.highlight_capture_names().iter().filter(|capture| {
                !capture.starts_with('_')
                    && !capture.starts_with("injection.")
                    // The provider emits no run for these captures.
                    && !matches!(**capture, "spell" | "nospell" | "conceal")
            }) {
                // Built-in definitions need not cover every name: a missing
                // one takes its nearest defined dotted ancestor's appearance.
                assert!(
                    resolve_syntax_name(&sheet, &canonical_capture_name(capture)).is_some(),
                    "no appearance for {language}: @{capture}"
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
            .get_mut(&invalid.base_paragraph)
            .unwrap()
            .role = BlockRole::Document;
        assert!(parse_json(&export_snapshot(&invalid).unwrap()).is_err());
    }

    #[test]
    fn persisted_names_suppression_and_validation_are_exact() {
        let mut sheet = default_sheet();
        let id = resolve_name(&sheet, "Keyword").unwrap().clone();
        sheet.character_metadata.get_mut(&id).unwrap().display_name = "Custom keyword".into();
        let removed = resolve_name(&sheet, "Todo").unwrap().clone();
        sheet.remove_character_style(&removed, false).unwrap();
        let encoded = export_snapshot(&sheet).unwrap();
        let restored = parse_json(&encoded).unwrap();
        assert!(resolve_name(&restored, "Keyword").is_none());
        assert!(resolve_name(&restored, "Todo").is_none());
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
            name: SyntaxStyleName("Keyword".into()),
            origin: "rust:@keyword".into(),
            priority: 100,
        };
        let mut sheet = default_sheet();
        let id = resolve_name(&sheet, "Keyword").unwrap().clone();
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
        sheet.character_styles.get_mut(&id).unwrap().properties.size = Some(24.0.into());
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
        let parent = next.character_styles.get(id).and_then(|s| s.based_on.clone());
        for child in next.character_styles.values_mut().filter(|s| s.based_on.as_ref() == Some(id)) {
            child.based_on = parent.clone();
        }
    }
    next.apply_configuration_edit(&edit, revision, false)
        .map_err(|e| format!("{e:?}"))?;
    // Any edit makes an implicit definition an ordinary persisted one.
    next.implicit_characters.remove(edit.style_id());
    validate(&next)?;
    *guard = Arc::new(next);
    Ok(guard.clone())
}
