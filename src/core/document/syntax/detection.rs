//! Versioned, declarative Vim-inspired detection. Markers select a language;
//! they never run Vimscript or configure arbitrary editor options.
use super::SyntaxInputSnapshot;
use std::collections::BTreeSet;
mod profile;
pub use profile::{
    bundled_profile, ContentMatch, ContentSignature, DetectionProfile, ExtensionDisambiguator,
    CONTENT_LITERAL_BYTE_LIMIT, CONTENT_RULE_LIMIT,
};

pub const DETECTION_BYTE_LIMIT: usize = 64 * 1024;
pub const PROFILE_VERSION: u32 = 1;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LanguageSelection {
    Automatic,
    None,
    Language(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Detection {
    pub language: Option<String>,
    pub reason: String,
    pub bytes_inspected: usize,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FilenameAssociation {
    pub pattern: String,
    pub language: String,
}

pub fn canonical_language(language: &str) -> String {
    match language {
        "cs" | "csharp" | "c#" => "c_sharp",
        "c++" => "cpp",
        "javascriptreact" | "jsx" | "js" => "javascript",
        "typescriptreact" => "tsx",
        "ts" => "typescript",
        "objective-c" | "objectivec" => "objc",
        "py" => "python",
        "sh" | "zsh" | "shell" => "bash",
        value => value,
    }
    .to_owned()
}

pub fn detect(
    input: &SyntaxInputSnapshot,
    filename: &str,
    selection: &LanguageSelection,
    associations: &[FilenameAssociation],
) -> Detection {
    detect_with_profile(input, filename, selection, associations, &bundled_profile())
}

/// Uses one validated declarative policy and the same bounded input samples as
/// modelines and shebang detection; it does not execute package code.
pub fn detect_with_profile(
    input: &SyntaxInputSnapshot,
    filename: &str,
    selection: &LanguageSelection,
    associations: &[FilenameAssociation],
    profile: &DetectionProfile,
) -> Detection {
    let result = |language: Option<String>, reason: &str, bytes| Detection {
        language,
        reason: reason.into(),
        bytes_inspected: bytes,
    };
    match selection {
        LanguageSelection::None => return result(None, "explicit none", 0),
        LanguageSelection::Language(value) => {
            return result(Some(canonical_language(value)), "explicit language", 0)
        }
        _ => {}
    }
    let text = input.text_tree();
    let count = text.hard_line_count();
    let mut lines = BTreeSet::new();
    lines.extend(0..count.min(5));
    lines.extend(count.saturating_sub(5)..count);
    let mut inspected = 0;
    let mut samples = Vec::new();
    let half = DETECTION_BYTE_LIMIT / 2;
    let head_end = input.byte_len().min(half);
    let tail_start = input.byte_len().saturating_sub(half);
    // Complete lines only: a limit-truncated marker must never select a language.
    for line in lines {
        let start = text.hard_line_start(line).unwrap_or(0);
        let end = text.hard_line_end(line).unwrap_or(start);
        if !((line < 5 && end <= head_end)
            || (line >= count.saturating_sub(5) && start >= tail_start))
        {
            continue;
        }
        if let Ok(value) = input.slice(start..end) {
            inspected += value.len();
            samples.push((line, value));
        }
    }
    let mut modeline = None;
    for (_, line) in &samples {
        if let Some(language) = marker(line) {
            modeline = Some(canonical_language(&language));
        }
    }
    if modeline.is_some() {
        return result(modeline, "Vim modeline", inspected);
    }
    let (content_matches, _) = profile.scan(&samples);
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    if base.len() <= 4096 {
        let characters = base.chars().collect::<Vec<_>>();
        let mut glob_fuel = 64 * 1024;
        for association in associations.iter().take(256) {
            if glob_fuel == 0 {
                break;
            }
            if association.pattern.len() <= 256
                && glob(&association.pattern, &characters, &mut glob_fuel)
            {
                return result(
                    Some(canonical_language(&association.language)),
                    "user filename association",
                    inspected,
                );
            }
        }
        let extension = base.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
        if let Some(language) = profile.extension(extension, content_matches) {
            return result(
                Some(canonical_language(language)),
                "content extension disambiguator",
                inspected,
            );
        }
        let language = match base {
            "Makefile" | "makefile" | "GNUmakefile" => Some("make"),
            "Dockerfile" => Some("dockerfile"),
            "CMakeLists.txt" => Some("cmake"),
            _ => match extension {
                "c" => Some("c"),
                "h" => Some("c"),
                "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" | "C" | "H" => Some("cpp"),
                "m" => Some("objc"),
                "mm" => Some("objc"),
                "rs" => Some("rust"),
                "swift" => Some("swift"),
                "cs" => Some("c_sharp"),
                "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
                "ts" | "mts" | "cts" => Some("typescript"),
                "tsx" => Some("tsx"),
                "py" | "pyw" | "pyi" => Some("python"),
                "sh" | "bash" | "zsh" => Some("bash"),
                "vim" => Some("vim"),
                "go" => Some("go"),
                "java" => Some("java"),
                "rb" => Some("ruby"),
                "lua" => Some("lua"),
                "pl" | "pm" => Some("perl"),
                "php" => Some("php"),
                "json" | "jsonc" => Some("json"),
                "yaml" | "yml" => Some("yaml"),
                "toml" => Some("toml"),
                "xml" => Some("xml"),
                "css" | "scss" | "sass" => Some("css"),
                "sql" => Some("sql"),
                "conf" => Some("conf"),
                "ini" => Some("dosini"),
                "md" | "markdown" | "mkd" => Some("markdown"),
                "html" | "htm" | "xhtml" => Some("html"),
                "rtf" => Some("rtf"),
                _ => None,
            },
        };
        if let Some(language) = language {
            return result(
                Some(language.into()),
                "bundled filename association",
                inspected,
            );
        }
    }
    if let Some((_, first)) = samples.iter().find(|(line, _)| *line == 0) {
        if let Some(language) = shebang(first) {
            return result(Some(language), "shebang", inspected);
        }
    }
    if let Some(language) = profile.signature(content_matches) {
        return result(
            Some(canonical_language(language)),
            "content signature",
            inspected,
        );
    }
    result(None, "unknown", inspected)
}

fn marker(line: &str) -> Option<String> {
    let mut found = None;
    let (mut active, mut first, mut set_form) = (false, false, false);
    // Visit each byte once even for a line containing thousands of markers.
    // The colon ending a `set` form also ends its allowed option declarations.
    let mut previous_was_space = true;
    for piece in line.split_inclusive(|ch: char| ch.is_whitespace() || ch == ':') {
        let colon = piece.ends_with(':');
        let item = piece.trim_end_matches(|ch: char| ch.is_whitespace() || ch == ':');
        if colon && previous_was_space && matches!(item, "vim" | "vi" | "ex") {
            active = true;
            first = true;
            set_form = false;
        } else if active && !item.is_empty() {
            if first && matches!(item, "set" | "se") {
                set_form = true;
            } else if let Some(value) = item
                .strip_prefix("ft=")
                .or_else(|| item.strip_prefix("filetype="))
            {
                if !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_+.-".contains(&b))
                {
                    found = Some(value.to_owned());
                }
            }
            first = false;
            if colon && set_form {
                active = false;
            }
        }
        previous_was_space = piece.ends_with(char::is_whitespace);
    }
    found
}

fn shebang(line: &str) -> Option<String> {
    let body = line.strip_prefix("#!")?.trim();
    let mut words = body.split_whitespace();
    let mut interpreter = words.next()?.rsplit('/').next()?;
    if interpreter == "env" {
        interpreter = "";
        let mut skip_argument = false;
        for word in words {
            if skip_argument {
                skip_argument = false;
                continue;
            }
            if matches!(word, "-u" | "--unset" | "-C" | "--chdir") {
                skip_argument = true;
                continue;
            }
            if word.starts_with('-') || word.contains('=') {
                continue;
            }
            interpreter = word.trim_matches(['\'', '"']);
            break;
        }
    }
    Some(
        if interpreter.starts_with("python") {
            "python"
        } else {
            match interpreter {
                "node" | "nodejs" | "deno" | "bun" => "javascript",
                "ruby" => "ruby",
                "perl" => "perl",
                "sh" | "bash" | "zsh" | "fish" => "bash",
                "lua" | "luajit" => "lua",
                "swift" => "swift",
                _ => return None,
            }
        }
        .into(),
    )
}

fn glob(pattern: &str, v: &[char], fuel: &mut usize) -> bool {
    let p = pattern.chars().collect::<Vec<_>>();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < v.len() {
        if *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        if i < p.len() && (p[i] == '?' || p[i] == v[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        if *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        i += 1;
    }
    i == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{syntax::SyntaxInputIdentity, FormattedTextTree};
    fn input(text: &str) -> SyntaxInputSnapshot {
        SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 1,
                revision: 0,
                generation: 1,
            },
            FormattedTextTree::try_from_text(text).unwrap(),
        )
    }
    #[test]
    fn markers_aliases_and_precedence() {
        let x = input("// vim: ft=rust\ntext\n// vim: set filetype=typescriptreact:");
        assert_eq!(
            detect(&x, "x.py", &LanguageSelection::Automatic, &[])
                .language
                .as_deref(),
            Some("tsx")
        );
        assert_eq!(
            detect(&x, "x.py", &LanguageSelection::None, &[]).language,
            None
        );
        assert_eq!(
            detect(
                &input("# vim: ft=unknown123"),
                "x.py",
                &LanguageSelection::Automatic,
                &[]
            )
            .language
            .as_deref(),
            Some("unknown123")
        );
    }
    #[test]
    fn bounded_giant_lines_and_env() {
        let x = input(&format!("{}vim: ft=rust", "x".repeat(4 * 1024 * 1024)));
        let r = detect(&x, "untitled", &LanguageSelection::Automatic, &[]);
        assert!(r.bytes_inspected <= DETECTION_BYTE_LIMIT);
        assert_eq!(r.language, None);
        let x = input("#!/usr/bin/env -u HOME -S PYTHONUTF8=1 python3 -I\nprint(1)");
        assert_eq!(
            detect(&x, "script", &LanguageSelection::Automatic, &[])
                .language
                .as_deref(),
            Some("python")
        );
    }
    #[test]
    fn modelines_are_bounded_data_and_only_load_time_declarations() {
        assert_eq!(
            marker("vim: set ft=rust: filetype=python").as_deref(),
            Some("rust")
        );
        assert_eq!(marker("prefixvim: ft=rust"), None);
        assert_eq!(marker("vim: syn=rust syntax=python ft=../../escape"), None);
        assert_eq!(
            marker("vim: ft=python\tvi: set filetype=swift:").as_deref(),
            Some("swift")
        );
        let repeated = format!("{} ft=unknown", "vim: ".repeat(5000));
        assert_eq!(marker(&repeated).as_deref(), Some("unknown"));
        let text = format!(
            "{}\n{}\n# vim: ft=cs",
            "// middle\n".repeat(6),
            "x".repeat(100_000)
        );
        let result = detect(&input(&text), "x.py", &LanguageSelection::Automatic, &[]);
        assert_eq!(result.language.as_deref(), Some("c_sharp"));
        assert!(result.bytes_inspected <= DETECTION_BYTE_LIMIT);
        let associations = [FilenameAssociation {
            pattern: "*.h".into(),
            language: "rust".into(),
        }];
        assert_eq!(
            detect(
                &input("class Foo {};"),
                "foo.h",
                &LanguageSelection::Automatic,
                &associations
            )
            .language
            .as_deref(),
            Some("rust")
        );
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::document::{syntax::SyntaxInputIdentity, FormattedTextTree};
    fn input(text: &str) -> SyntaxInputSnapshot {
        SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 99,
                revision: 1,
                generation: 1,
            },
            FormattedTextTree::try_from_text(text).unwrap(),
        )
    }
    fn signature(language: &str, literal: &str, matching: ContentMatch) -> ContentSignature {
        ContentSignature {
            language: language.into(),
            literal: literal.into(),
            matching,
        }
    }
    #[test]
    fn registered_rules_share_bounded_samples_and_preserve_precedence() {
        let mut profile = DetectionProfile::default();
        profile
            .register_signature(signature("custom", "MAGIC", ContentMatch::FirstLinePrefix))
            .unwrap();
        profile
            .register_disambiguator(ExtensionDisambiguator {
                extension: "h".into(),
                signature: signature("custom_header", "custom ", ContentMatch::Contains),
            })
            .unwrap();
        let detect = |text: &str, filename: &str, associations: &[FilenameAssociation]| {
            detect_with_profile(
                &input(text),
                filename,
                &LanguageSelection::Automatic,
                associations,
                &profile,
            )
        };
        assert_eq!(
            detect("MAGIC payload", "unknown", &[]).language.as_deref(),
            Some("custom")
        );
        assert_eq!(
            detect("MAGIC payload", "x.py", &[]).language.as_deref(),
            Some("python")
        );
        assert_eq!(
            detect("#!/usr/bin/python3 MAGIC", "unknown", &[])
                .language
                .as_deref(),
            Some("python")
        );
        assert_eq!(
            detect("custom class X;", "x.h", &[]).language.as_deref(),
            Some("custom_header")
        );
        assert_eq!(
            detect(
                "custom class X;",
                "x.h",
                &[FilenameAssociation {
                    pattern: "*.h".into(),
                    language: "user".into()
                }]
            )
            .language
            .as_deref(),
            Some("user")
        );
        assert_eq!(
            detect("custom // vim: ft=rust", "x.h", &[])
                .language
                .as_deref(),
            Some("rust")
        );
        assert_eq!(
            detect("ordinary", "x.h", &[]).language.as_deref(),
            Some("c")
        );
        assert_eq!(
            detect("ordinary", "x.m", &[]).language.as_deref(),
            Some("objc")
        );
        assert_eq!(
            detect("  % comment", "x.m", &[]).language.as_deref(),
            Some("matlab")
        );
        assert_eq!(
            detect("class X;", "x.h", &[]).language.as_deref(),
            Some("cpp")
        );
        assert_eq!(
            detect("  <?xml version", "unknown", &[])
                .language
                .as_deref(),
            Some("xml")
        );
        let giant = format!(
            "{}MAGIC\n{}\ncustom ",
            "x".repeat(4 * 1024 * 1024),
            "middle\n".repeat(6)
        );
        let result = detect(&giant, "x.h", &[]);
        assert_eq!(result.language.as_deref(), Some("custom_header"));
        assert!(result.bytes_inspected <= DETECTION_BYTE_LIMIT);
        let result = detect(&giant, "unknown", &[]);
        assert_eq!(result.language, None, "truncated first line cannot match");
    }
    #[test]
    fn registration_and_automaton_work_have_numeric_bounds() {
        let rules = (1..=CONTENT_RULE_LIMIT)
            .map(|length| signature("fixture", &"a".repeat(length), ContentMatch::Contains))
            .collect();
        let mut profile = DetectionProfile::compile(rules, vec![]).unwrap();
        let samples = vec![(0, "a".repeat(DETECTION_BYTE_LIMIT))];
        let (found, steps) = profile.scan(&samples);
        assert_eq!(found, u64::MAX);
        assert!(steps <= 2 * DETECTION_BYTE_LIMIT + CONTENT_RULE_LIMIT);
        assert!(profile.states_len_for_test() <= CONTENT_LITERAL_BYTE_LIMIT + 1);
        assert!(profile
            .register_signature(signature("overflow", "x", ContentMatch::Contains))
            .is_err());
        assert_eq!(
            profile.scan(&samples).0,
            found,
            "failed registration must preserve policy"
        );
        let oversized = (0..17)
            .map(|_| signature("fixture", &"a".repeat(256), ContentMatch::Contains))
            .collect();
        assert!(DetectionProfile::compile(oversized, vec![]).is_err());
        assert!(DetectionProfile::compile(
            vec![signature("../escape", "x", ContentMatch::Contains)],
            vec![]
        )
        .is_err());
        let prefix = DetectionProfile::compile(
            vec![signature("prefix", "a", ContentMatch::FirstLinePrefix)],
            vec![],
        )
        .unwrap();
        assert_eq!(prefix.scan(&[(0, "xa a a a".into())]).0, 0);
        assert_eq!(prefix.scan(&[(0, "   a".into())]).0, 1);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::{
        document::{Document, Encoding, FileFormat, Format},
        layout::MockTextMeasurementProvider,
        Core,
    };
    use std::sync::Arc;
    #[test]
    fn installed_profile_redetects_without_source_changes_and_respects_explicit_none() {
        let document = Document::from_bytes_with_file_format(
            b"MAGIC payload".to_vec(),
            Encoding::Utf8,
            Format::Code,
            FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::<MockTextMeasurementProvider>::new(document);
        core.initialize_code_detection("unknown", false).unwrap();
        assert_eq!(core.code_language_detection().unwrap().language, None);
        let revision = core.document().revision();
        let mut profile = DetectionProfile::default();
        profile
            .register_signature(ContentSignature {
                language: "custom".into(),
                literal: "MAGIC".into(),
                matching: ContentMatch::FirstLinePrefix,
            })
            .unwrap();
        core.set_code_detection_profile(Arc::new(profile));
        assert_eq!(
            core.code_language_detection().unwrap().language.as_deref(),
            Some("custom")
        );
        assert_eq!(core.document().revision(), revision);
        core.set_code_language(LanguageSelection::None);
        core.set_code_detection_profile(bundled_profile());
        assert_eq!(core.code_language_detection().unwrap().language, None);
        assert_eq!(core.document().revision(), revision);
    }
}
