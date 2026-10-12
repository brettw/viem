//! Formatting defaults, independent of language lexical support.
//!
//! References: Vim v9.2.0000, runtime/ftplugin/*.vim. Exact effective upstream
//! values, conditional choices, and deliberate adaptations are recorded in
//! docs/comment-language-coverage.md. Nothing executes or loads Vimscript here.

const C_DOC: &str = ":///,://!,://,s1:/*,mb:*,ex:*/";
const C_BLOCK_LINE: &str = "s1:/*,mb:*,ex:*/,://";
const C_BLOCK: &str = "s1:/*,mb:*,ex:*/";
const HTML_BLOCK: &str = "s:<!--,m:    ,e:-->";

/// Canonical catalogue identities only. A profile is not a lexical support claim.
pub const LANGUAGE_COMMENT_PROFILES: &[(&str, &str)] = &[
    ("astro", HTML_BLOCK),
    ("bash", ":#"),
    ("c", C_DOC),
    ("c_sharp", C_DOC),
    ("cmake", "s:#[[,m: ,e:]],:#"),
    ("cpp", C_DOC),
    ("css", C_BLOCK),
    ("dart", ":///,://,s1:/*,mb:*,ex:*/"),
    ("dash", ":#"),
    ("dockerfile", ":#"),
    ("dosbatch", "b:rem,b:@rem,b:REM,b:@REM,:::"),
    ("dosini", ":;,:#"),
    ("gitcommit", ":#"),
    ("go", C_BLOCK_LINE),
    ("graphql", ":#"),
    ("groovy", C_BLOCK_LINE),
    ("haskell", "s1fl:{-,mb:-,ex:-},:--"),
    ("hcl", "s1:/*,mb:*,ex:*/,://,:#"),
    ("html", HTML_BLOCK),
    ("java", ":///,://,s1:/*,mb:*,ex:*/"),
    ("javascript", C_BLOCK_LINE),
    ("json", ""),
    ("json5", C_BLOCK_LINE),
    ("jsonc", C_BLOCK_LINE),
    ("kotlin", C_BLOCK_LINE),
    ("ksh", ":#"),
    ("lua", "s:--[[,m: ,e:]],:---,:--"),
    ("make", ":#"),
    ("markdown", HTML_BLOCK),
    ("mermaid", ":%%"),
    ("mksh", ":#"),
    ("objc", C_DOC),
    ("perl", ":#"),
    ("php", "s1:/*,mb:*,ex:*/,://,:#"),
    ("plaintex", ":%"),
    ("proto", C_BLOCK_LINE),
    ("ps1", "s:<#,m: ,ex:#>,:#"),
    ("python", ":#"),
    ("r", ":#',:###,:##,:#"),
    ("ruby", ":#"),
    ("rust", "s:/*!,m: ,ex:*/,s1:/*,mb:*,ex:*/,:///,://!,://"),
    ("scala", "s1:/**,mb:*,ex:*/,s1:/*,mb:*,ex:*/,://"),
    ("scss", C_BLOCK_LINE),
    ("sh", ":#"),
    ("sql", "s1:/*,mb:*,ex:*/,:--"),
    ("swift", ":///,://,s1:/*,mb:*,ex:*/"),
    ("terraform", "s1:/*,mb:*,ex:*/,://,:#"),
    ("tex", ":%"),
    ("toml", ":#"),
    ("tsx", C_BLOCK_LINE),
    ("typescript", C_BLOCK_LINE),
    ("typst", C_BLOCK_LINE),
    ("vim", ":\",:#"),
    ("vue", HTML_BLOCK),
    ("xml", HTML_BLOCK),
    ("yaml", ":#"),
    ("zsh", ":#"),
];

pub fn default_comments(canonical_language: &str) -> &'static str {
    LANGUAGE_COMMENT_PROFILES
        .iter()
        .find_map(|&(language, profile)| (language == canonical_language).then_some(profile))
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::super::{CommentProfile, ReflowOptions};
    use super::*;

    #[test]
    fn defaults_are_valid_unique_canonical_catalogue_entries() {
        let catalogue = crate::document::syntax::languages::supported_languages();
        let mut seen = std::collections::BTreeSet::new();
        for &(language, value) in LANGUAGE_COMMENT_PROFILES {
            assert!(seen.insert(language), "duplicate {language}");
            assert!(
                catalogue.iter().any(|entry| entry.id == language),
                "missing catalogue entry {language}"
            );
            assert!(CommentProfile::parse(value).is_ok(), "{language}: {value}");
        }
    }

    #[test]
    fn language_changes_preserve_replacements_including_empty_and_equal_defaults() {
        for replacement in ["", ":#", C_DOC] {
            let mut options = ReflowOptions::default();
            options.set_language(Some("c++"));
            assert_eq!(options.language(), Some("cpp"));
            assert_eq!(options.comments(), C_DOC);
            options.comments = Some(replacement.into());
            for language in [Some("py"), Some("unknown"), None, Some("cpp")] {
                options.set_language(language);
                assert_eq!(options.comments(), replacement);
            }
            options.comments = None;
            options.set_language(Some("py"));
            assert_eq!(options.language(), Some("python"));
            assert_eq!(options.comments(), ":#");
            options.set_language(None);
            assert_eq!(options.comments(), "");
        }
    }

    #[test]
    fn defaults_distinguish_css_and_scss_and_keep_nonsemantic_lists_out() {
        assert!(CommentProfile::parse(default_comments("css"))
            .unwrap()
            .line_leaders
            .is_empty());
        assert_eq!(
            CommentProfile::parse(default_comments("scss"))
                .unwrap()
                .line_leaders[0]
                .text,
            "//"
        );
        assert!(!default_comments("python").contains("fb:-"));
        assert!(!default_comments("markdown").contains("fb:"));
        assert_eq!(default_comments("json"), "");
    }
}
