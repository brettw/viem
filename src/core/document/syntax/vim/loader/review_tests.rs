use super::super::*;
use super::logical_lines;

#[test]
fn continuation_bytes_and_heredoc_comments_preserve_native_commands() {
    let source = "syn match Joined /\\\n  \\%(a\\|b\\)\\\n  \\+/\nsyn keyword Spaced \n  \\word\nlet s:lines =<< trim END \" a comment\n  first\n  second\nEND\nif join(s:lines) !=# 'first second'\n  unsupported heredoc\nendif\n";
    let statements = logical_lines(source).unwrap();
    assert_eq!(statements[0].text, "syn match Joined /\\%(a\\|b\\)\\+/");
    assert_eq!(statements[1].text, "syn keyword Spaced word");
    let program = compile("continuation.vim", source, VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(), 2);
}

#[test]
fn highlight_queries_follow_declarations_and_generated_global_helpers() {
    let program = compile("groups.vim", r#"
if !hlexists('Normal') || hlexists('String') || hlexists('Unseen')
  unsupported initial groups
endif
hi Unseen
hi link Unlinked NONE
if hlexists('Unseen') || hlexists('NONE') || !hlexists('Unlinked')
  unsupported query or NONE registration
endif
syn match Defined /a/ contains=Referenced nextgroup=Next
syn region Box matchgroup=Edge start=/x/ end=/y/
hi def link Linked Target
if !hlexists('defined') || !hlexists('Referenced') || !hlexists('Next') || !hlexists('Edge') || !hlexists('Linked') || !hlexists('Target')
  unsupported missing groups
endif
function NativeDefine(name)
  execute 'syn keyword '.a:name.' generated'
endfunction
call NativeDefine('Generated')
if !hlexists('Generated')
  unsupported missing generated group
endif
"#, VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.last().unwrap().group, "Generated");
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "viem-loader-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn caught_setup_error_skips_remaining_try_body_and_restores_exception() {
    let program = compile(
        "try.vim",
        r#"
try
  let s:value = missing
  syn keyword Wrong wrong
catch /E121:/
  if v:exception !~# '^E121:'
    unsupported exception state
  endif
  syn keyword Right right
finally
  syn keyword Final final
endtry
if exists('v:exception')
  unsupported leaked exception
endif
"#,
        VimLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(
        program
            .rules
            .iter()
            .map(|rule| rule.group.as_str())
            .collect::<Vec<_>>(),
        ["Right", "Final"]
    );
}

#[test]
fn nested_catches_restore_outer_exception_and_do_not_hide_unsupported_calls() {
    let program = compile(
        "try.vim",
        r#"
try
  let s:value = missing
catch /E121:/
  try
    unlet g:absent
    syn keyword Wrong wrong
  catch /E108:/
    syn keyword Inner inner
  endtry
  if v:exception !~# '^E121:'
    unsupported lost outer exception
  endif
  syn keyword Outer outer
endtry
"#,
        VimLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(program.rules.len(), 2);
    assert!(compile("try.vim", "try\nlet x = missing\ncatch /E121:/\ncall system('date')\nfinally\nsyn keyword Final final\nendtry", VimLoadLimits::default()).is_err());
    assert!(compile(
        "try.vim",
        "try\ncall system('date')\ncatch\nsyn keyword Wrong wrong\nendtry",
        VimLoadLimits::default()
    )
    .is_err());
    assert!(compile(
        "try.vim",
        "let v:exception = 'fake'",
        VimLoadLimits::default()
    )
    .is_err());
}

#[test]
fn expanded_nonlocal_control_flow_is_diagnosed() {
    for control in ["break", "continue", "finish"] {
        let source =
            format!("for item in [1,2]\nexecute '{control}'\nsyn keyword Wrong wrong\nendfor");
        let errors = compile("expanded.vim", &source, VimLoadLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("nonlocal control flow")),
            "{errors:?}"
        );
    }
    // A loop entirely inside one expansion owns its break normally.
    let program = compile(
        "expanded.vim",
        "execute 'for item in [1,2] | break | endfor'\nsyn keyword Good good",
        VimLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(program.rules.len(), 1);
}

#[test]
fn runtime_globs_charge_one_shared_budget_and_reject_unsupported_patterns() {
    let directory = Directory::new();
    for name in ["a", "b", "c", "d"] {
        std::fs::write(directory.0.join(name), "").unwrap();
    }
    let mut loader = Loader::new(
        VimLoadLimits::default(),
        Some(directory.0.canonicalize().unwrap()),
    );
    loader.statement_fuel = 10;
    assert!(loader
        .runtime("missing*.vim missing*.vim missing*.vim", true)
        .unwrap_err()
        .contains("budget"));
    let mut loader = Loader::new(
        VimLoadLimits::default(),
        Some(directory.0.canonicalize().unwrap()),
    );
    assert!(loader
        .runtime("[ab].vim", true)
        .unwrap_err()
        .contains("unsupported"));
}

#[cfg(unix)]
#[test]
fn runtime_globs_reject_directory_symlinks_before_external_enumeration() {
    let directory = Directory::new();
    let outside = Directory::new();
    std::fs::write(
        outside.0.join("external.vim"),
        "syn keyword Outside outside",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside.0, directory.0.join("escape")).unwrap();
    let mut loader = Loader::new(
        VimLoadLimits::default(),
        Some(directory.0.canonicalize().unwrap()),
    );
    assert!(loader
        .runtime("escape/missing*.vim", true)
        .unwrap_err()
        .contains("escapes configured"));
    assert_eq!(loader.files, 0);
    assert!(loader.program.rules.is_empty());
}
