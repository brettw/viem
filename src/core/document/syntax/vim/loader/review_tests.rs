use super::super::*;
use crate::vimscript::source::script_lines;

#[test]
fn continuation_bytes_and_heredoc_comments_preserve_native_commands() {
    let source = "syn match Joined /\\\n  \\%(a\\|b\\)\\\n  \\+/\nsyn keyword Spaced \n  \\word\nlet s:lines =<< trim END \" a comment\n  first\n  second\nEND\nif join(s:lines) !=# 'first second'\n  unsupported heredoc\nendif\n";
    let statements = script_lines(source.as_bytes());
    assert_eq!(statements[0].text.to_str_lossy(), "syn match Joined /\\%(a\\|b\\)\\+/");
    assert_eq!(statements[1].text.to_str_lossy(), "syn keyword Spaced word");
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
fn shared_values_functions_and_declarations_are_synchronous() {
    let program = compile("identity.vim", r#"
let s:values = []
let s:alias = s:values
let s:object = {'items': s:values}
function s:object.Define(name) dict
  call add(self.items, a:name)
  execute 'syn keyword '.a:name.' token'
endfunction
call s:object.Define ('Generated')
call map(s:alias, {i, value -> value . 'Again'})
if s:values isnot s:alias || s:values[0] !=# 'GeneratedAgain' || !hlexists('Generated')
  unsupported identities or execution order
endif
let s:captured = execute('syntax list @Missing', 'silent!')
let s:lines =<< trim eval END
  {s:values[0]}
  "literal | comment
END
if s:lines[0] !=# 'GeneratedAgain' || s:lines[1] !=# '"literal | comment'
  unsupported heredoc
endif
syn keyword Finished finished
"#, VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.iter().map(|r|r.group.as_str()).collect::<Vec<_>>(), ["Generated", "Finished"]);
}

#[test]
fn syntax_vim9_declarations_use_script_scope_without_locking_globals() {
    let program=compile("scope.vim", "vim9script\nconst value = 1\ng:value = 2\nif value != 1 || g:value != 2\nthrow 'wrong scope'\nendif\nsyn keyword Good good\n", VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(),1);
}

#[test]
fn autoload_uses_the_same_scopes_and_exists_does_not_source_it() {
    let directory = Directory::new();
    let syntax = directory.0.join("syntax");
    std::fs::create_dir(&syntax).unwrap();
    std::fs::create_dir(directory.0.join("autoload")).unwrap();
    std::fs::write(directory.0.join("autoload/helper.vim"), r#"
let g:helper#value = 'Loaded'
let g:loads = get(g:, 'loads', 0) + 1
function helper#Define() abort
  execute 'syn keyword '.g:helper#value.' token'
endfunction
"#).unwrap();
    std::fs::write(syntax.join("owner.vim"), r#"
if exists('g:helper#value') || exists('*helper#Define') || exists('g:loads')
  unsupported eager autoload
endif
call helper#Define()
call helper#Define()
if g:loads != 1 || g:helper#value !=# 'Loaded'
  unsupported autoload scope
endif
"#).unwrap();
    let program = directory_with_context(&syntax, "owner", VimLoadLimits::default(), &VimSetupContext::default(), None).unwrap();
    assert_eq!(program.source_files.len(), 2);
    assert_eq!(program.rules.len(), 2);
}

#[test]
fn confined_imports_support_escaped_paths_and_bounded_globs() {
    let directory=Directory::new();
    std::fs::write(directory.0.join("child file.vim"),"syn keyword Child child\n").unwrap();
    std::fs::write(directory.0.join("owner.vim"),"execute 'source ' . fnameescape(expand('<sfile>:p:h').'/child file.vim')\n").unwrap();
    let program=directory_with_context(&directory.0,"owner",VimLoadLimits::default(),&VimSetupContext::default(),None).unwrap();
    assert_eq!(program.rules.len(),1);
    assert_eq!(program.rules[0].group,"Child");
    for (name,group) in [("a.vim","A"),("b.vim","B")] {
        std::fs::write(directory.0.join(name),format!("syn keyword {group} token\n")).unwrap();
    }
    let mut loader=Loader::new(VimLoadLimits::default(),Some(directory.0.canonicalize().unwrap()));
    loader.runtime("[ab].vim",true).unwrap();
    assert_eq!(loader.finish().unwrap().rules.len(),2);
    let mut loader=Loader::new(VimLoadLimits::default(),Some(directory.0.canonicalize().unwrap()));
    loader.source("outside.vim","try\ncall globpath('/', '*')\ncatch\nendtry\n");
    assert!(loader.finish().is_err());
}

#[test]
fn conceal_character_tokens_keep_spaces_and_delimiter_characters() {
    let program = compile("conceal.vim", "syn match Space /x/ conceal cchar= \nsyn match Pipe /y/ conceal cchar=|\nsyn match Quote /z/ conceal cchar=\"\nsyn match Composed /a/ conceal cchar=x̶\n", VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(), 4);
    assert!(compile("invalid.vim", "syn match X /x/ conceal cchar=", VimLoadLimits::default()).is_err());
}

#[test]
fn query_cursor_and_highlight_attributes_are_private_and_synchronous() {
    let mut loader=Loader::new(VimLoadLimits::default(),None);
    loader.setup=Setup::new(&VimSetupContext {prefix:"first\nsecond\n".into(),filename:None,input:None},None);
    loader.source("queries.vim", r#"
call cursor(2, 1)
set ignorecase
if 'first' !~ 'FIRST'
  unsupported private option
endif
if line('.') != 2 || search('FIRST', 'w') != 1 || getpos('.') != [0,1,1,0]
  unsupported private cursor
endif
hi Foo ctermfg=Blue ctermbg=DarkRed guifg=red cterm=bold,underline
hi link Bar Foo
if synIDattr(hlID('Foo'),'fg','cterm') !=# '9' || synIDattr(hlID('Foo'),'bg','cterm') !=# '4'
  unsupported terminal attributes
endif
if synIDattr(synIDtrans(hlID('Bar')),'name') !=# 'Foo' || synIDattr(hlID('Foo'),'fg#','gui') !=# '#ff0000'
  unsupported linked attributes
endif
syn keyword Done done
"#);
    assert_eq!(loader.finish().unwrap().rules.len(),1);
}

#[test]
fn literal_word_union_partition_preserves_matches_and_limits() {
    let words = (0..2500).map(|n|format!("CONST_{n}")).collect::<Vec<_>>();
    let source = format!("syn match Constant /#\\<\\%({}\\)\\>/", words.join("\\|"));
    let program = compile("constants.vim", &source, VimLoadLimits::default()).unwrap();
    assert!(program.rules.len() > 1);
    for word in ["#CONST_0", "#CONST_250", "#CONST_2499", "#CONST_2500", "#CONST_250x"] {
        let mut count = 0;
        for rule in &program.rules {
            let RuleKind::Match(pattern) = &rule.kind else {panic!()};
            count += usize::from(pattern.is_match_text(word, 1_000_000).unwrap());
        }
        assert_eq!(count, usize::from(matches!(word,"#CONST_0"|"#CONST_250"|"#CONST_2499")), "{word}");
    }
    // Without the word-character proof, two alternatives might match at the
    // same position and partitioning would change ordered-choice semantics.
    assert!(compile("ambiguous.vim", &format!("set isk-=_\n{source}"), VimLoadLimits::default()).is_err());
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
  if v:exception !~# 'E121:'
    unsupported exception state
  endif
  syn keyword Right right
finally
  syn keyword Final final
endtry
if v:exception !=# ''
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
  if v:exception !~# 'E121:'
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
fn unsupported_commands_cannot_spoof_a_catchable_missing_file_error() {
    let errors = compile(
        "try.vim",
        "try\nunsupported No such file or directory\ncatch /E484:/\nsyn keyword Wrong wrong\nendtry",
        VimLoadLimits::default(),
    ).unwrap_err();
    assert!(errors.iter().any(|error| error.message.contains("unsupported")));
}

#[test]
fn expanded_control_flow_uses_shared_frame_boundaries() {
    for control in ["break", "continue"] {
        let source = format!("for item in [1,2]\nexecute '{control}'\nsyn keyword Wrong wrong\nendfor");
        assert!(compile("expanded.vim", &source, VimLoadLimits::default()).is_err());
    }
    let program=compile("expanded.vim", "execute 'finish'\nsyn keyword Wrong wrong", VimLoadLimits::default()).unwrap();
    assert!(program.rules.is_empty());
    let program=compile("expanded.vim", "execute 'for item in [1,2] | break | endfor'\nsyn keyword Good good", VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(),1);
}

#[test]
fn runtime_globs_charge_one_shared_budget() {
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

// Exercise the same resource import path used by :runtime without introducing
// another script evaluator in loader tests.
impl Loader<'_> {
    fn runtime(&mut self, pattern: &str, all: bool) -> Result<(), String> {
        let mut rt = self.new_script_runtime();
        let result = self.script_import(&mut rt, pattern, all).map_err(|e|format!("{e:?}"));
        self.script_runtime = Some(rt);
        result
    }
}

#[test]
fn vim9_typed_helpers_and_confined_modules_share_the_evaluator() {
    let directory = Directory::new();
    let syntax = directory.0.join("syntax");
    let imports = directory.0.join("import");
    std::fs::create_dir(&syntax).unwrap();
    std::fs::create_dir(&imports).unwrap();
    std::fs::write(imports.join("groups.vim"), "vim9script\nexport def Define(name: string, words: list<string>)\n  execute 'syn keyword ' .. name .. ' ' .. join(words)\nenddef\n").unwrap();
    std::fs::write(syntax.join("owner.vim"), "vim9script\nimport '../import/groups.vim' as groups\nvar words = ['first', 'second']\ngroups.Define('Generated', words)\n").unwrap();
    let program = directory_with_context(&syntax, "owner", VimLoadLimits::default(), &VimSetupContext::default(), None).unwrap();
    assert_eq!(program.source_files.len(), 2);
    assert!(program.rules.iter().all(|rule|rule.group == "Generated"));
    assert!(!program.rules.is_empty());
    std::fs::write(directory.0.join("outside.vim"), "vim9script\nexport var escaped = 1\n").unwrap();
    std::fs::write(syntax.join("escape.vim"), "vim9script\ntry\n  import '../outside.vim' as outside\ncatch\nendtry\n").unwrap();
    assert!(directory_with_context(&syntax, "escape", VimLoadLimits::default(), &VimSetupContext::default(), None).is_err());
}
