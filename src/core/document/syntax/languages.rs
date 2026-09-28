//! The bundled language catalogue, independent of the host and lazy providers.
use std::{collections::BTreeSet, sync::OnceLock};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Language {
    pub id: String,
    pub name: String,
}

pub fn display_name(id: &str) -> String {
    match id {
        "c" => "C", "cpp" => "C++", "c_sharp" => "C#", "objc" => "Objective-C",
        "javascript" => "JavaScript", "typescript" => "TypeScript", "tsx" => "TSX",
        "html" => "HTML", "xml" => "XML", "json" => "JSON", "json5" => "JSON5",
        "jsonc" => "JSON with Comments", "css" => "CSS", "scss" => "SCSS",
        "sql" => "SQL", "php" => "PHP", "yaml" => "YAML", "toml" => "TOML",
        "dosini" => "INI", "dosbatch" => "Windows Batch", "ps1" => "PowerShell",
        "vim" => "Vim Script", "make" => "Makefile", "cmake" => "CMake",
        "bash" => "Bash", "sh" => "Shell", "zsh" => "Zsh", "go" => "Go",
        "rust" => "Rust", "swift" => "Swift", "python" => "Python",
        "markdown" => "Markdown", "tex" => "TeX", "plaintex" => "Plain TeX",
        value => return value.chars().next().map(|first| first.to_uppercase().to_string() + &value[first.len_utf8()..]).unwrap_or_default(),
    }.into()
}

pub fn supported_languages() -> &'static [Language] {
    static CATALOGUE: OnceLock<Vec<Language>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../../../assets/vim/manifest.json")).expect("bundled Vim manifest");
        let mut ids = BTreeSet::new();
        for path in manifest["files"].as_object().expect("Vim file inventory").keys() {
            let Some(id) = path.strip_prefix("runtime/syntax/").and_then(|name| name.strip_suffix(".vim")) else { continue };
            // Runtime entry points and conversion utilities are not languages.
            if id.contains('/') || matches!(id, "2html" | "manual" | "nosyntax" | "syncolor" | "synload" | "syntax") { continue; }
            ids.insert(super::detection::canonical_language(id));
        }
        // Tree-sitter also supplies TSX, which uses TypeScript for Vim fallback.
        ids.insert("tsx".into());
        let mut result = ids.into_iter().map(|id| Language { name: display_name(&id), id }).collect::<Vec<_>>();
        result.sort_by_key(|language| (language.name.to_lowercase(), language.id.clone()));
        result
    })
}
