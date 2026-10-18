//! Language context, formatting overrides, and comment input share one policy.
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, Format};

fn code(source: &str, language: &str) -> (CommandInterpreter, Document) {
    let mut commands = CommandInterpreter::new();
    commands.set_reflow_language(Some(language.into()));
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    (commands, document)
}

fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) -> CommandStatus {
    commands
        .handle(document, InputEvent::Key(key))
        .unwrap()
        .status
}

fn keys(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
    for character in text.chars() {
        let status = key(commands, document, Key::Char(character));
        assert!(
            !matches!(status, CommandStatus::Error(_) | CommandStatus::ExError(_)),
            "{text:?}: {status:?}"
        );
    }
}

fn set(commands: &mut CommandInterpreter, document: &mut Document, options: &str) {
    keys(commands, document, &format!(":set {options}"));
    let status = key(commands, document, Key::Enter);
    assert!(
        !matches!(status, CommandStatus::Error(_) | CommandStatus::ExError(_)),
        "{options}: {status:?}"
    );
}

#[test]
fn trailing_line_enter_open_below_and_open_above_have_distinct_rules() {
    for (language, source, leader, column) in [
        ("cpp", "int x; // explanation", "//", 7),
        ("python", "x = 1 # explanation", "#", 6),
        ("lua", "x = 1 -- explanation", "--", 6),
    ] {
        for (operation, slash, continued) in [
            ("enter", false, true),
            ("enter", true, true),
            ("o", false, true),
            ("o", true, false),
            ("O", false, false),
        ] {
            let (mut commands, mut document) = code(source, language);
            set(
                &mut commands,
                &mut document,
                if slash { "fo=qjro/" } else { "fo=qjro" },
            );
            if operation == "enter" {
                keys(&mut commands, &mut document, "A");
                key(&mut commands, &mut document, Key::Enter);
            } else {
                keys(&mut commands, &mut document, operation);
            }
            let expected = if operation == "O" {
                format!("\n{source}")
            } else if continued {
                format!("{source}\n{}{leader} \n", " ".repeat(column))
            } else {
                format!("{source}\n\n")
            };
            assert_eq!(
                document.text(),
                expected,
                "{language} {operation} slash={slash}"
            );
            key(&mut commands, &mut document, Key::Escape);
            keys(&mut commands, &mut document, "u");
            assert_eq!(document.text(), source, "one input undo group");
        }
    }
}

#[test]
fn trailing_block_enter_uses_the_block_column_and_closed_blocks_do_not_continue() {
    for (source, expected) in [
        ("x; /* body", "x; /* body\n    * \n"),
        ("x; /* body */", "x; /* body */\n\n"),
    ] {
        let (mut commands, mut document) = code(source, "cpp");
        set(&mut commands, &mut document, "fo+=r");
        keys(&mut commands, &mut document, "A");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), expected);
    }
}

#[test]
fn protected_multiline_literals_do_not_generate_or_strip_configured_leaders() {
    for (language, source, marker_line) in [
        (
            "cpp",
            "auto text = R\"tag(\n// first\n// second\n)tag\";",
            1,
        ),
        ("python", "text = \"\"\"\n# first\n# second\n\"\"\"", 1),
        ("lua", "text = [=[\n-- first\n-- second\n]=]", 1),
    ] {
        for operation in ["J", "o"] {
            let (mut commands, mut document) = code(source, language);
            set(&mut commands, &mut document, "fo=qjro");
            commands.set_cursor(&document, document.line_start(marker_line).unwrap());
            keys(&mut commands, &mut document, operation);
            if operation == "J" {
                let joined = match language {
                    "cpp" => "// first // second",
                    "python" => "# first # second",
                    _ => "-- first -- second",
                };
                assert!(
                    document.text().contains(joined),
                    "{language}: {}",
                    document.text()
                );
            } else {
                assert!(
                    document.text().contains("first\n\n"),
                    "{language}: {}",
                    document.text()
                );
            }
        }
    }
}

#[test]
fn manual_leaders_customize_code_but_cannot_redefine_strings() {
    let (mut commands, mut document) = code("# custom", "cpp");
    set(&mut commands, &mut document, "comments=:# fo+=ro");
    keys(&mut commands, &mut document, "o");
    assert_eq!(document.text(), "# custom\n# \n");

    let (mut commands, mut document) = code("\"quoted text\"", "cpp");
    // As in Vim, an unescaped `"` starts a comment after `:set`.
    set(&mut commands, &mut document, "comments=:\\\" fo+=ro");
    keys(&mut commands, &mut document, "o");
    assert_eq!(document.text(), "\"quoted text\"\n\n");

    let (mut commands, mut document) = code("// TODO: investigate", "cpp");
    set(&mut commands, &mut document, r"comments=://\ TODO: fo+=ro");
    keys(&mut commands, &mut document, "o");
    assert_eq!(document.text(), "// TODO: investigate\n// TODO: \n");
}

#[test]
fn generated_closing_keeps_the_originating_block_with_shared_middle_and_suffix() {
    for (opening, closing) in [("#{", "#}"), ("@{", "@}")] {
        let (mut commands, mut document) = code(opening, "cpp");
        set(
            &mut commands,
            &mut document,
            "comments=s1:#{,mb:#,ex:#},s1:@{,mb:#,ex:@} fo+=r",
        );
        keys(&mut commands, &mut document, "A");
        key(&mut commands, &mut document, Key::Enter);
        keys(&mut commands, &mut document, "}");
        assert_eq!(document.text(), format!("{opening}\n {closing}\n"));
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), opening);
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(document.text(), format!("{opening}\n {closing}\n"));
    }
}

#[test]
fn whitespace_middle_still_generates_its_configured_closing() {
    let (mut commands, mut document) = code("#{", "cpp");
    set(
        &mut commands,
        &mut document,
        r"comments=s1:#{,m:\ ,ex:#} fo+=r",
    );
    keys(&mut commands, &mut document, "A");
    key(&mut commands, &mut document, Key::Enter);
    keys(&mut commands, &mut document, "}");
    assert_eq!(document.text(), "#{\n #}\n");
}

#[test]
fn generated_closing_is_retired_by_language_or_source_changes() {
    for change in ["language", "source"] {
        let (mut commands, mut document) = code("/*", "cpp");
        set(&mut commands, &mut document, "fo+=r");
        keys(&mut commands, &mut document, "A");
        key(&mut commands, &mut document, Key::Enter);
        if change == "language" {
            commands.set_reflow_language(Some("rust".into()));
            commands.set_reflow_language(Some("cpp".into()));
        } else {
            document.insert(document.text().len(), "tail").unwrap();
        }
        keys(&mut commands, &mut document, "/");
        assert!(
            document.text().starts_with("/*\n * /\n"),
            "{change}: {}",
            document.text()
        );
    }
}

#[test]
fn repeating_a_generated_close_outside_a_comment_keeps_the_typed_character() {
    let (mut commands, mut document) = code("/*", "cpp");
    set(&mut commands, &mut document, "fo+=r");
    keys(&mut commands, &mut document, "A");
    key(&mut commands, &mut document, Key::Enter);
    keys(&mut commands, &mut document, "/");
    key(&mut commands, &mut document, Key::Escape);
    assert_eq!(document.text(), "/*\n */\n");
    // Create a later code target without replacing the captured Insert recipe
    // or suggesting an authored unstarred convention inside the first block.
    document.insert(document.text().len(), "code").unwrap();
    assert_eq!(document.text(), "/*\n */\ncode");
    keys(&mut commands, &mut document, "G.");
    assert_eq!(document.text(), "/*\n */\ncode\n/\n");
    keys(&mut commands, &mut document, "u");
    assert_eq!(document.text(), "/*\n */\ncode");
}

#[test]
fn block_split_before_closer_continues_without_generating_a_second_closer() {
    let (mut commands, mut document) = code("/* body */", "cpp");
    set(&mut commands, &mut document, "fo+=r");
    commands.set_cursor(&document, 8);
    keys(&mut commands, &mut document, "i");
    key(&mut commands, &mut document, Key::Enter);
    assert_eq!(document.text(), "/* body \n * */");
    keys(&mut commands, &mut document, "/");
    assert_eq!(document.text(), "/* body \n * /*/");
}

#[test]
fn override_modifiers_freeze_the_profile_and_reset_restores_actual_language_behavior() {
    let (mut commands, mut document) = code("# python", "cpp");
    set(&mut commands, &mut document, "comments+=:@ fo+=o");
    commands.set_reflow_language(Some("python".into()));
    keys(&mut commands, &mut document, "o");
    assert_eq!(document.text(), "# python\n\n");
    key(&mut commands, &mut document, Key::Escape);
    keys(&mut commands, &mut document, "u");
    set(&mut commands, &mut document, "comments&");
    keys(&mut commands, &mut document, "o");
    assert_eq!(document.text(), "# python\n# \n");
}

#[test]
fn counted_comment_open_and_dot_recompute_each_new_context_in_one_undo_unit() {
    let (mut commands, mut document) = code("# first\n# second", "python");
    set(&mut commands, &mut document, "fo+=o");
    keys(&mut commands, &mut document, "2ox");
    key(&mut commands, &mut document, Key::Escape);
    assert_eq!(document.text(), "# first\n# x\n# x\n# second");
    keys(&mut commands, &mut document, "G.");
    assert_eq!(document.text(), "# first\n# x\n# x\n# second\n# x\n# x\n");
    keys(&mut commands, &mut document, "uu");
    assert_eq!(document.text(), "# first\n# second");
}

#[test]
fn unknown_context_reflow_is_atomic_and_input_and_join_use_ordinary_behavior() {
    // This literal line exceeds the bounded lexical provider's line budget.
    let source = format!("{}\n// first\n// second", "x".repeat(300_000));
    for operation in ["J", "o", "enter", "gq", "gw"] {
        let (mut commands, mut document) = code(&source, "cpp");
        set(&mut commands, &mut document, "fo=qjro");
        commands.set_cursor(&document, document.line_start(1).unwrap());
        let revision = document.revision();
        if matches!(operation, "gq" | "gw") {
            keys(&mut commands, &mut document, "Vj");
            let cursor = commands.cursor();
            let anchor = commands.visual_anchor();
            let mode = commands.mode();
            let register = commands.register('"').cloned();
            key(&mut commands, &mut document, Key::Char('g'));
            let status = key(
                &mut commands,
                &mut document,
                Key::Char(operation.chars().last().unwrap()),
            );
            assert!(
                matches!(status, CommandStatus::Error(_)),
                "{operation}: {status:?}"
            );
            assert_eq!(document.revision(), revision);
            assert_eq!(document.text(), source);
            assert_eq!(commands.cursor(), cursor);
            assert_eq!(commands.visual_anchor(), anchor);
            assert_eq!(commands.mode(), mode);
            assert_eq!(commands.register('"'), register.as_ref());
        } else {
            if operation == "enter" {
                keys(&mut commands, &mut document, "A");
                key(&mut commands, &mut document, Key::Enter);
            } else {
                keys(&mut commands, &mut document, operation);
            }
            if operation == "J" {
                assert!(document.text().ends_with("// first // second"));
            } else {
                assert!(document.text().ends_with("// first\n\n// second"));
            }
        }
    }
}

#[test]
fn native_language_families_share_reflow_join_and_literal_protection_policy() {
    for (language, literal, leader) in [
        ("c", "char *s = \"// literal\";", "//"),
        ("objc", "id s = @\"// literal\";", "//"),
        ("rust", "let s = r###\"// literal\"###;", "//"),
        ("go", "s := `// literal`", "//"),
        ("java", "String s = \"// literal\";", "//"),
        ("javascript", "const s = '// literal';", "//"),
        ("typescript", "const s: string = '// literal';", "//"),
        ("tsx", "const s = '// literal';", "//"),
        ("c_sharp", "var s = @\"// literal\";", "//"),
        ("swift", "let s = \"// literal\"", "//"),
        ("kotlin", "val s = \"// literal\"", "//"),
        ("scala", "val s = \"// literal\"", "//"),
        ("groovy", "def s = '// literal'", "//"),
        ("dart", "var s = '// literal';", "//"),
        ("proto", "string s = '// literal';", "//"),
        ("scss", "$s: '// literal';", "//"),
        ("jsonc", "\"s\": \"// literal\",", "//"),
        ("json5", "s: '// literal',", "//"),
        ("python", "s = '# literal'", "#"),
        ("r", "s <- '# literal'", "#"),
        ("toml", "s = '# literal'", "#"),
        ("graphql", "\"# literal\"", "#"),
        ("hcl", "s = \"# literal\"", "#"),
        ("terraform", "s = \"# literal\"", "#"),
        ("bash", "s='# literal'", "#"),
        ("sh", "s='# literal'", "#"),
        ("zsh", "s='# literal'", "#"),
        ("ksh", "s='# literal'", "#"),
        ("dash", "s='# literal'", "#"),
        ("mksh", "s='# literal'", "#"),
        ("yaml", "s: '# literal'", "#"),
        ("ps1", "$s = '# literal'", "#"),
        ("cmake", "set(s \"# literal\")", "#"),
        ("lua", "s = [=[-- literal]=]", "--"),
        ("sql", "SELECT '-- literal';", "--"),
        ("haskell", "s = \"-- literal\"", "--"),
        ("dosini", "s=\"; literal\"", ";"),
        ("tex", r"\% literal", "%"),
        ("plaintex", r"\% literal", "%"),
        ("vim", "let s = '\" literal'", "\""),
    ] {
        let source = format!("{literal}\n{leader} alpha beta\n{leader} gamma delta");
        for operation in ["gqG", "gwG", "J", ":2,3join"] {
            let (mut commands, mut document) = code(&source, language);
            commands.set_cursor(&document, document.line_start(1).unwrap());
            keys(&mut commands, &mut document, operation);
            if operation.starts_with(':') {
                assert_eq!(
                    key(&mut commands, &mut document, Key::Enter),
                    CommandStatus::Complete
                );
            }
            assert_eq!(
                document.text(),
                format!("{literal}\n{leader} alpha beta gamma delta"),
                "{language}: {operation}"
            );
            keys(&mut commands, &mut document, "u");
            assert_eq!(document.text(), source, "{language}: undo {operation}");
        }
        let (mut commands, mut document) = code(&source, language);
        keys(&mut commands, &mut document, "J");
        assert_eq!(
            document.text(),
            format!("{literal} {leader} alpha beta\n{leader} gamma delta"),
            "{language}: literal delimiter must not authorize leader removal"
        );
    }
}

#[test]
fn compatibility_profiles_keep_explicit_literal_editing_behavior() {
    // These languages have defaults, but this test deliberately makes no
    // promise that their strings or embedded languages are lexically protected.
    for (language, leader) in [
        ("perl", "#"),
        ("ruby", "#"),
        ("php", "//"),
        ("dockerfile", "#"),
        ("make", "#"),
        ("gitcommit", "#"),
        ("dosbatch", "rem"),
        ("mermaid", "%%"),
        ("typst", "//"),
    ] {
        let source = format!("{leader} alpha beta\n{leader} gamma delta");
        let (mut commands, mut document) = code(&source, language);
        keys(&mut commands, &mut document, "gqG");
        assert_eq!(
            document.text(),
            format!("{leader} alpha beta gamma delta"),
            "{language}"
        );
        keys(&mut commands, &mut document, "u");
        set(&mut commands, &mut document, "fo+=ro");
        keys(&mut commands, &mut document, "o");
        assert_eq!(
            document.text(),
            format!("{leader} alpha beta\n{leader} \n{leader} gamma delta"),
            "{language}"
        );
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "u");
        set(&mut commands, &mut document, "comments=");
        keys(&mut commands, &mut document, "ggJ");
        assert_eq!(
            document.text(),
            format!("{leader} alpha beta {leader} gamma delta"),
            "{language}: empty profile"
        );
    }
}

#[test]
fn unsupported_forms_stay_unknown_instead_of_using_literal_defaults() {
    for (language, prelude, leader) in [
        ("javascript", "const re = /[//]/;", "//"),
        ("python", "s = f'{nested}'", "#"),
        ("c_sharp", "var s = $\"{nested}\";", "//"),
        ("swift", "let s = #\"raw\"#", "//"),
        ("scala", "val s = s\"${nested}\"", "//"),
        ("scss", "$s: #{nested};", "//"),
        ("hcl", "s = <<EOF", "#"),
        ("bash", "s=$(echo nested)", "#"),
        ("ps1", "$s = @\"", "#"),
    ] {
        let source = format!("{prelude}\n{leader} alpha beta\n{leader} gamma delta");
        let (mut commands, mut document) = code(&source, language);
        set(&mut commands, &mut document, "fo+=ro");
        commands.set_cursor(&document, document.line_start(1).unwrap());
        let revision = document.revision();
        keys(&mut commands, &mut document, "gq");
        assert!(
            matches!(
                key(&mut commands, &mut document, Key::Char('G')),
                CommandStatus::Error(_)
            ),
            "{language}"
        );
        assert_eq!(document.revision(), revision);
        keys(&mut commands, &mut document, "o");
        assert_eq!(
            document.text(),
            format!("{prelude}\n{leader} alpha beta\n\n{leader} gamma delta"),
            "{language}"
        );
    }
}

#[test]
fn heredocs_scalars_and_multiline_strings_protect_leaders_across_commands() {
    for (language, source, marker) in [
        ("bash", "cat <<'EOF'\n# first\n# second\nEOF", "#"),
        ("sh", "cat <<-EOF\n\t# first\n\t# second\n\tEOF", "#"),
        ("yaml", "s: |\n  # first\n  # second", "#"),
        ("toml", "s = '''\n# first\n# second\n'''", "#"),
        ("java", "var s = \"\"\"\n// first\n// second\n\"\"\";", "//"),
        ("go", "s := `\n// first\n// second\n`", "//"),
        ("rust", "let s = r##\"\n// first\n// second\n\"##;", "//"),
        ("cmake", "set(s [=[\n# first\n# second\n]=])", "#"),
    ] {
        for operation in ["gqj", "gwj", "J", ":2,3join"] {
            let (mut commands, mut document) = code(source, language);
            commands.set_cursor(&document, document.line_start(1).unwrap());
            keys(&mut commands, &mut document, operation);
            if operation.starts_with(':') {
                key(&mut commands, &mut document, Key::Enter);
            }
            if operation.starts_with('g') {
                assert_eq!(document.text(), source, "{language}: {operation}");
            } else {
                assert!(
                    document
                        .text()
                        .contains(&format!("{marker} first {marker} second")),
                    "{language}: {operation}: {}",
                    document.text()
                );
            }
        }
    }
}

#[test]
fn block_comment_families_reflow_join_and_continue_the_authored_middle() {
    for (language, open, middle, close) in [
        ("css", "/*", " * ", " */"),
        ("rust", "/*!", " ", "*/"),
        ("lua", "--[[", " ", "]]"),
        ("cmake", "#[[", " ", "]]"),
        ("haskell", "{-", " - ", " -}"),
        ("ps1", "<#", " ", "#>"),
        ("xml", "<!--", "     ", "-->"),
        ("html", "<!--", "     ", "-->"),
    ] {
        let source = format!("{open} alpha beta\n{middle}gamma delta\n{close}");
        for operation in ["gqG", "gwG", "J", ":1,2join"] {
            let (mut commands, mut document) = code(&source, language);
            keys(&mut commands, &mut document, operation);
            if operation.starts_with(':') {
                key(&mut commands, &mut document, Key::Enter);
            }
            assert_eq!(
                document.text(),
                format!("{open} alpha beta gamma delta\n{close}"),
                "{language}: {operation}"
            );
            keys(&mut commands, &mut document, "u");
            assert_eq!(document.text(), source);
        }
        for operation in ["o", "O"] {
            let (mut commands, mut document) = code(&source, language);
            set(&mut commands, &mut document, "fo+=o");
            commands.set_cursor(&document, document.line_start(1).unwrap());
            keys(&mut commands, &mut document, operation);
            let expected = if operation == "o" {
                format!("{open} alpha beta\n{middle}gamma delta\n{middle}\n{close}")
            } else {
                format!("{open} alpha beta\n{middle}\n{middle}gamma delta\n{close}")
            };
            assert_eq!(document.text(), expected, "{language}: {operation}");
        }
    }
}

#[test]
fn dynamic_long_blocks_require_a_matching_profile_and_nested_comments_resume_the_outer_rule() {
    for (language, open, close) in [("lua", "--[==[", "]==]"), ("cmake", "#[==[", "]==]")] {
        let source = format!("{open} alpha beta\n gamma delta\n{close}");
        let (mut commands, mut document) = code(&source, language);
        keys(&mut commands, &mut document, "gqG");
        assert_eq!(
            document.text(),
            source,
            "{language}: unmatched native delimiter remains unchanged"
        );
        set(
            &mut commands,
            &mut document,
            &format!(r"comments=s:{open},m:\ ,e:{close}"),
        );
        keys(&mut commands, &mut document, "gggqG");
        assert_eq!(
            document.text(),
            format!("{open} alpha beta gamma delta\n{close}")
        );
    }
    for language in ["rust", "swift", "kotlin", "scala", "dart"] {
        let source = "/* outer\n /* inner */\n * tail";
        let (mut commands, mut document) = code(source, language);
        set(&mut commands, &mut document, "fo+=o");
        keys(&mut commands, &mut document, "Go");
        assert_eq!(
            document.text(),
            "/* outer\n /* inner */\n * tail\n * \n",
            "{language}"
        );
        keys(&mut commands, &mut document, "/");
        assert_eq!(
            document.text(),
            "/* outer\n /* inner */\n * tail\n */\n",
            "{language}"
        );
    }
}

#[test]
fn manual_block_profiles_cannot_redefine_a_native_closing_delimiter() {
    for operation in ["gqG", "J", "o"] {
        let source = "/* first\n * second\n */";
        let (mut commands, mut document) = code(source, "cpp");
        set(
            &mut commands,
            &mut document,
            "comments=s1:/*,mb:*,ex:END fo+=ro",
        );
        keys(&mut commands, &mut document, operation);
        let expected = match operation {
            "gqG" => source,
            "J" => "/* first * second\n */",
            _ => "/* first\n\n * second\n */",
        };
        assert_eq!(document.text(), expected, "{operation}");
    }
}

#[test]
fn shell_parameters_escaped_word_boundaries_and_splices_are_not_comments() {
    for (source, line) in [
        ("echo $# ${#x}", 0),
        ("echo word\\ #not-comment", 0),
        ("echo word\\;#not-comment", 0),
        ("echo word\\\n#not-comment", 1),
    ] {
        let (mut commands, mut document) = code(source, "bash");
        set(&mut commands, &mut document, "fo+=ro");
        commands.set_cursor(&document, document.line_start(line).unwrap());
        keys(&mut commands, &mut document, "o");
        assert_eq!(document.text(), format!("{source}\n\n"), "{source:?}");
    }
}

#[test]
fn continuation_selects_the_comment_at_the_caret_on_lines_with_multiple_owners() {
    for (language, source, prefix) in [
        ("cpp", "/* done */ // active", "           // "),
        ("rust", "/* outer /* inner */ tail", " * "),
        ("swift", "/* outer /* inner */ tail", " * "),
    ] {
        let (mut commands, mut document) = code(source, language);
        set(&mut commands, &mut document, "fo+=r");
        keys(&mut commands, &mut document, "A");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            document.text(),
            format!("{source}\n{prefix}\n"),
            "{language}"
        );
    }
    let (mut commands, mut document) = code("/* done */ // active\n// continuation", "cpp");
    keys(&mut commands, &mut document, "J");
    assert_eq!(document.text(), "/* done */ // active continuation");
}

#[test]
fn counted_custom_block_joins_share_context_in_logical_and_physical_commands() {
    use viem_core::command::LineMode;
    let body = (0..64)
        .map(|index| format!(" # word{index}\n"))
        .collect::<String>();
    let source = format!("#{{\n{body} #}}\ntail");
    let words = (0..64)
        .map(|index| format!("word{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    for physical in [false, true] {
        for command in ["65J", ":1,65join"] {
            let (mut commands, mut document) = code(&source, "cpp");
            set(&mut commands, &mut document, "comments=s1:#{,mb:#,ex:#}");
            if physical {
                commands
                    .set_line_mode(&document, LineMode::PhysicalSource)
                    .unwrap();
            }
            keys(&mut commands, &mut document, command);
            if command.starts_with(':') {
                key(&mut commands, &mut document, Key::Enter);
            }
            assert_eq!(
                document.text(),
                format!("#{{ {words}\n #}}\ntail"),
                "physical={physical}: {command}"
            );
            keys(&mut commands, &mut document, "u");
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn generated_closing_publishes_the_final_inserted_text_in_the_last_insert_register() {
    for operation in ["enter", "o"] {
        let (mut commands, mut document) = code("/*", "cpp");
        set(&mut commands, &mut document, "fo+=ro");
        if operation == "enter" {
            keys(&mut commands, &mut document, "A");
            key(&mut commands, &mut document, Key::Enter);
        } else {
            keys(&mut commands, &mut document, "o");
        }
        keys(&mut commands, &mut document, "/");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "/*\n */\n");
        let inserted = commands.register('.').unwrap();
        assert_eq!(
            inserted.text,
            if operation == "enter" { "\n */" } else { " */" }
        );
        assert_eq!(
            inserted.hard_break_offsets(),
            if operation == "enter" {
                &[0][..]
            } else {
                &[][..]
            }
        );
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "/*");
    }
}

#[test]
fn later_custom_regions_own_the_caret_after_closed_native_or_custom_blocks() {
    for (source, profile, continuation) in [
        ("/* old */ # custom", ":#", "          # "),
        (
            "#{old#} @{ live",
            "s1:#{,mb:#,ex:#},s1:@{,mb:@,ex:@}",
            "         @ ",
        ),
        ("/* old */ @{ live", "s1:@{,mb:@,ex:@}", "           @ "),
    ] {
        let (mut commands, mut document) = code(source, "cpp");
        set(
            &mut commands,
            &mut document,
            &format!("comments={profile} fo+=r"),
        );
        keys(&mut commands, &mut document, "A");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), format!("{source}\n{continuation}\n"));
    }
}

#[test]
fn each_ordered_block_rule_can_reflow_its_own_closed_comment() {
    for (language, source) in [
        ("rust", "/* alpha beta gamma delta */"),
        ("scala", "/* alpha beta gamma delta */"),
    ] {
        let (mut commands, mut document) = code(source, language);
        set(&mut commands, &mut document, "tw=15");
        keys(&mut commands, &mut document, "gqq");
        assert_eq!(document.text(), "/* alpha beta\n * gamma\n * delta */");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), source);
    }
}

#[test]
fn a_native_profile_cannot_generate_text_past_an_earlier_native_closer() {
    let (mut commands, mut document) = code("/* body", "cpp");
    set(
        &mut commands,
        &mut document,
        "comments=s1:/*,mb:*,ex:*/oops*/ fo+=r",
    );
    keys(&mut commands, &mut document, "A");
    key(&mut commands, &mut document, Key::Enter);
    assert_eq!(document.text(), "/* body\n\n");
}

#[test]
fn compatibility_context_also_bounds_current_line_work() {
    let source = format!("# {}", "x".repeat(300_000));
    let (mut commands, mut document) = code(&source, "perl");
    set(&mut commands, &mut document, "fo+=r");
    keys(&mut commands, &mut document, "A");
    key(&mut commands, &mut document, Key::Enter);
    assert_eq!(document.text(), format!("{source}\n\n"));
}
