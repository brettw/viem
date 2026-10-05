//! Pinned, unmodified nvim-treesitter queries and their inherited dependencies.
//! When adding/removing packages, follow PROFILE.md and ../detection/PROFILE.md:
//! keep grammar pins, BUNDLED_LANGUAGES/aliases, catalogue entries and filename
//! rules/tests in sync. Query helpers alone do not register languages/extensions.
use super::*;

fn source(id: &str, injection: bool) -> Result<&'static str, TreeSitterError> {
    let pair = match id {
        "c" => (include_str!("c.scm"), include_str!("c_injections.scm")),
        "cpp" => (include_str!("cpp.scm"), include_str!("cpp_injections.scm")),
        "rust" => (
            include_str!("rust.scm"),
            include_str!("rust_injections.scm"),
        ),
        "swift" => (
            include_str!("swift.scm"),
            include_str!("swift_injections.scm"),
        ),
        "objc" => (
            include_str!("objc.scm"),
            include_str!("objc_injections.scm"),
        ),
        "c_sharp" => (
            include_str!("c_sharp.scm"),
            include_str!("c_sharp_injections.scm"),
        ),
        "javascript" => (
            include_str!("javascript.scm"),
            include_str!("javascript_injections.scm"),
        ),
        "typescript" => (
            include_str!("typescript.scm"),
            include_str!("typescript_injections.scm"),
        ),
        "tsx" => (include_str!("tsx.scm"), include_str!("tsx_injections.scm")),
        "python" => (
            include_str!("python.scm"),
            include_str!("python_injections.scm"),
        ),
        "json" => (
            include_str!("json.scm"),
            include_str!("json_injections.scm"),
        ),
        "markdown" => (
            include_str!("markdown.scm"),
            include_str!("markdown_injections.scm"),
        ),
        "markdown_inline" => (
            include_str!("markdown_inline.scm"),
            include_str!("markdown_inline_injections.scm"),
        ),
        "ecma" => (
            include_str!("ecma.scm"),
            include_str!("ecma_injections.scm"),
        ),
        "jsx" => (include_str!("jsx.scm"), include_str!("jsx_injections.scm")),
        _ => return Err(TreeSitterError::UnsupportedLanguage(id.to_owned())),
    };
    Ok(if injection { pair.1 } else { pair.0 })
}

// Dependencies precede the child, and each shared query is included once.
// This mirrors Neovim's recursive `inherits` order (notably TSX -> TS -> ECMA).
fn compose(id: &str, injection: bool) -> Result<String, TreeSitterError> {
    fn append(
        id: &str,
        injection: bool,
        seen: &mut BTreeSet<String>,
        output: &mut String,
    ) -> Result<(), TreeSitterError> {
        if !seen.insert(id.to_owned()) {
            return Ok(());
        }
        let text = source(id, injection)?;
        for line in text.lines() {
            if let Some(parents) = line.strip_prefix("; inherits:") {
                for parent in parents.split(',') {
                    append(parent.trim(), injection, seen, output)?;
                }
            }
        }
        for line in text.lines().filter(|line| !line.starts_with("; inherits:")) {
            output.push_str(line);
            output.push('\n');
        }
        Ok(())
    }
    let mut output = String::new();
    append(id, injection, &mut BTreeSet::new(), &mut output)?;
    Ok(output)
}

pub(super) fn package(id: &str) -> Result<Arc<TreeSitterPackage>, TreeSitterError> {
    let canonical = match id {
        "cs" => "c_sharp",
        "javascriptreact" => "javascript",
        "typescriptreact" => "tsx",
        _ => id,
    };
    let language: Language = match canonical {
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "objc" => tree_sitter_objc::LANGUAGE.into(),
        "c_sharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "markdown" => tree_sitter_md::LANGUAGE.into(),
        "markdown_inline" => tree_sitter_md::INLINE_LANGUAGE.into(),
        _ => return Err(TreeSitterError::UnsupportedLanguage(id.to_owned())),
    };
    TreeSitterPackage::compile(
        id,
        1,
        language,
        &compose(canonical, false)?,
        Some(&compose(canonical, true)?),
        QueryProfile::NeovimV1,
        &TreeSitterBudget::default(),
        None,
    )
}
