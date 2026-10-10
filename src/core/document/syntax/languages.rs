//! The bundled language catalogue, independent of the host and lazy providers.
//! Language changes must also review canonical/provider aliases and filename rules
//! in detection/filenames.rs; see detection/PROFILE.md's maintenance checklist.
//! The Vim manifest supplies names, not extensions or a compatibility guarantee.
//! Languages without their own Vim entry point need explicit entries here; registering a grammar
//! does not automatically update this catalogue or filename detection.
use std::{collections::BTreeSet, sync::OnceLock};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Language {
    pub id: String,
    pub name: String,
    /// Portable menu policy shared by document and Markdown code-block pickers.
    pub primary: bool,
}

fn is_primary(id: &str) -> bool {
    matches!(id,
        "astro" | "bash" | "c" | "c_sharp" | "cmake" | "cpp" | "css" | "dart"
        | "diff" | "dockerfile" | "dosbatch" | "dosini" | "gitcommit" | "go"
        | "graphql" | "html" | "java" | "javascript" | "json" | "jsonc" | "kotlin"
        | "lua" | "make" | "markdown" | "mermaid" | "objc" | "php" | "ps1"
        | "python" | "r" | "ruby" | "rust" | "scss" | "sh" | "sql" | "swift"
        | "terraform" | "tex" | "toml" | "tsx" | "typescript" | "typst" | "vim"
        | "vue" | "xml" | "yaml")
}

pub fn display_name(id: &str) -> String {
    match id {
        "c" => "C", "cpp" => "C++", "c_sharp" => "C#", "objc" => "Objective-C",
        "javascript" => "JavaScript / JSX", "typescript" => "TypeScript", "tsx" => "TypeScript JSX (TSX)",
        "html" => "HTML", "xml" => "XML", "json" => "JSON", "json5" => "JSON5",
        "jsonc" => "JSON with Comments", "css" => "CSS", "scss" => "SCSS",
        "sql" => "SQL", "php" => "PHP", "yaml" => "YAML", "toml" => "TOML",
        "graphql" => "GraphQL", "hcl" => "HCL", "proto" => "Protocol Buffers",
        "dosini" => "INI", "dosbatch" => "Windows Batch", "ps1" => "PowerShell",
        "vim" => "Vim Script", "make" => "Makefile", "cmake" => "CMake",
        "bash" => "Bash", "sh" => "Shell", "zsh" => "Zsh", "go" => "Go",
        "ksh" => "KornShell", "dash" => "Dash", "mksh" => "mksh",
        "rust" => "Rust", "swift" => "Swift", "python" => "Python",
        "gitcommit" => "Git Commit",
        "markdown" => "Markdown", "tex" => "TeX / LaTeX", "plaintex" => "Plain TeX",
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
        // TSX uses its own Tree-sitter grammar and the React TypeScript Vim syntax.
        ids.insert("tsx".into());
        // sh.vim supplies these dialects through validated buffer setup flags.
        ids.extend(["ksh", "dash", "mksh"].map(str::to_owned));
        let mut result = ids.into_iter().map(|id| Language {
            name: display_name(&id), primary: is_primary(&id), id,
        }).collect::<Vec<_>>();
        result.sort_by_key(|language| (language.name.to_lowercase(), language.id.clone()));
        result
    })
}
