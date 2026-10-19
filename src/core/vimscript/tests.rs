//! Engine tests with an in-memory host. Fixture scripts under
//! `tests/all/fixtures/vimscript` have legacy output recorded from Vim 9.1
//! and Vim9 language output recorded from Vim 9.2.0000
//! (`scripts/vimscript-oracle.sh` regenerates them).
use super::error::{EvalResult, Fail, VimError};
use super::exec::{HostCommand, Message, OptionScope, RunState, Runtime, ScriptHost};
use super::state::ScriptState;
use super::value::Value;
use std::collections::BTreeMap;

/// A host with Vim-like options and registers and no buffer.
#[derive(Default)]
pub(crate) struct MemoryHost {
    pub output: String,
    pub options: BTreeMap<String, Value>,
    pub registers: BTreeMap<char, Value>,
    pub commands: Vec<String>,
    pub files: BTreeMap<String, Vec<u8>>,
    pub suspend_after: Option<usize>,
    executed: usize,
    last_context: Option<(Option<String>, Option<String>, Option<usize>)>,
}

impl MemoryHost {
    pub(crate) fn new() -> Self {
        let mut host = Self::default();
        for (name, value) in [("ignorecase", Value::Number(0)), ("tabstop", Value::Number(8)), ("shiftwidth", Value::Number(8)), ("encoding", Value::str("utf-8"))] {
            host.options.insert(name.into(), value);
        }
        host
    }
    fn canonical(name: &str) -> String {
        match name {
            "ic" => "ignorecase".into(),
            "ts" => "tabstop".into(),
            "sw" => "shiftwidth".into(),
            "enc" => "encoding".into(),
            other => other.into(),
        }
    }
}

impl ScriptHost for MemoryHost {
    fn execute(&mut self, _rt: &mut Runtime, command: &HostCommand) -> EvalResult<()> {
        self.executed += 1;
        self.commands.push(command.text.clone());
        match command.name {
            Some("set") | Some("setlocal") | Some("setglobal") => {
                for word in command.argument.split_whitespace() {
                    if let Some((name, value)) = word.split_once('=') {
                        let value = value.parse::<i64>().map(Value::Number).unwrap_or_else(|_| Value::str(value));
                        self.options.insert(Self::canonical(name), value);
                    } else if let Some(name) = word.strip_prefix("no") {
                        self.options.insert(Self::canonical(name), Value::Number(0));
                    } else {
                        self.options.insert(Self::canonical(word), Value::Number(1));
                    }
                }
                Ok(())
            }
            Some("normal") | Some("nohlsearch") | Some("map") | Some("nnoremap") | Some("highlight") | Some("syntax") | Some("filetype") => Ok(()),
            Some(name) => Err(Fail::Error(VimError::plain(format!("not supported in tests: {name}")))),
            None => Ok(()),
        }
    }
    fn get_option(&mut self, name: &str, _scope: OptionScope) -> Result<Value, VimError> {
        self.options.get(&Self::canonical(name)).cloned().ok_or_else(|| VimError::new(113, format!("Unknown option: {name}")))
    }
    fn set_option(&mut self, _rt: &mut Runtime, name: &str, _scope: OptionScope, value: &Value) -> EvalResult<()> {
        self.options.insert(Self::canonical(name), value.clone());
        Ok(())
    }
    fn option_exists(&self, name: &str, _working: bool) -> bool {
        self.options.contains_key(&Self::canonical(name))
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        Ok(self.registers.get(&name).cloned().unwrap_or_else(|| Value::str("")))
    }
    fn set_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError> {
        let value = if append {
            let mut text = self.registers.get(&name).map(|v| v.to_bytes().unwrap_or_default()).unwrap_or_default();
            text.extend(value.to_bytes().map_err(|e| e)?);
            Value::String(text.into())
        } else {
            value.clone()
        };
        self.registers.insert(name, value);
        Ok(())
    }
    fn call_function(&mut self, _rt: &mut Runtime, _name: &str, _args: &[Value]) -> Option<EvalResult<Value>> {
        None
    }
    fn has_function(&self, _name: &str) -> bool {
        false
    }
    fn feature(&self, _name: &str) -> Option<bool> {
        None
    }
    fn command_exists(&self, name: &str) -> bool {
        matches!(name, "set" | "setlocal" | "normal" | "nohlsearch" | "substitute")
    }
    fn message(&mut self, message: Message) {
        match message {
            Message::Echo(text) | Message::EchoMsg(text) | Message::Output(text) => {
                self.output.push('\n');
                self.output.push_str(&text);
            }
            Message::Echon(text) => self.output.push_str(&text),
            Message::Error(error) => {
                // Like Vim's msg_source(), repeat "line N:" only when the
                // context changed since the previous error.
                let context = (error.file.clone(), error.function.clone(), error.lnum);
                if let Some(lnum) = error.lnum {
                    if self.last_context.as_ref() != Some(&context) {
                        self.output.push_str(&format!("\nline {lnum}:"));
                    }
                }
                self.last_context = Some(context);
                self.output.push('\n');
                self.output.push_str(&error.error.text());
            }
        }
    }
    fn ignorecase(&self) -> bool {
        self.options.get("ignorecase").and_then(|v| v.to_number().ok()).unwrap_or(0) != 0
    }
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, VimError> {
        match self.files.get(path) {
            Some(bytes) => Ok(bytes.clone()),
            None => super::source::read_script_file(path),
        }
    }
    fn should_suspend(&mut self) -> bool {
        self.suspend_after.is_some_and(|limit| self.executed >= limit)
    }
}

/// Run a script and return its message output like Vim's `:redir`.
pub(crate) fn run_script(text: &str) -> String {
    let mut host = MemoryHost::new();
    run_with(&mut host, text);
    host.output
}

#[test]
fn locked_variables_can_be_unlet_and_named_calls_allow_whitespace() {
    let output = run_script("let x = [1]\nlockvar x\nunlet x\nfunction F(x)\nreturn a:x\nendfunction\ncall F (2)\necho exists('x') F (3)\n");
    assert_eq!(output, "\n0 3");
    let output = run_script("const x = 1\nunlet x\n");
    assert!(output.contains("E741:"), "{output}");
}

#[test]
fn skipped_expression_commands_keep_quoted_strings_and_bar_boundaries() {
    assert_eq!(run_script("if 0 | throw \"quoted | bar\" | endif\nif 0 | return \"quoted | bar\" | endif\necho 'done'\n"), "\ndone");
}

pub(crate) fn run_with(host: &mut MemoryHost, text: &str) -> Runtime {
    let mut rt = Runtime::new(ScriptState::new());
    rt.push_script("test.vim", text.as_bytes()).unwrap();
    assert_eq!(rt.run(host), RunState::Complete);
    rt
}

/// Normalize Vim's `:redir` capture: drop "Error detected" headers and
/// right-aligned line numbers.
pub(crate) fn normalize_vim_output(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.starts_with("Error detected while processing") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("line") {
            let trimmed = rest.trim_start();
            if trimmed.ends_with(':') && trimmed[..trimmed.len() - 1].chars().all(|c| c.is_ascii_digit()) && !trimmed[..trimmed.len() - 1].is_empty() {
                out.push(format!("line {trimmed}"));
                continue;
            }
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

fn fixture_directory() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/all/fixtures/vimscript")
}

#[test]
fn fixtures_match_vim_output() {
    let directory = fixture_directory();
    let mut entries: Vec<_> = std::fs::read_dir(&directory).expect("fixture directory").flatten().map(|entry| entry.path()).collect();
    entries.sort();
    let mut checked = 0;
    let mut failures = Vec::new();
    for path in entries {
        if path.extension().is_none_or(|extension| extension != "vim") {
            continue;
        }
        if let Ok(filter) = std::env::var("VIEM_VIMSCRIPT_FIXTURE") {
            if !path.to_string_lossy().contains(&filter) {
                continue;
            }
        }
        let script = std::fs::read_to_string(&path).unwrap();
        let expected_path = path.with_extension("out");
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|_| panic!("missing {}", expected_path.display()));
        let actual = normalize_vim_output(&run_script(&script));
        let expected = normalize_vim_output(&expected);
        if actual.trim_end() != expected.trim_end() {
            failures.push(format!("{}\n--- expected\n{}\n--- actual\n{}", path.display(), expected.trim_end(), actual.trim_end()));
        }
        checked += 1;
    }
    assert!(checked > 0, "no fixtures found");
    assert!(failures.is_empty(), "{} of {checked} fixtures differ:\n\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn smoke_echo_and_variables() {
    assert_eq!(run_script("let x = 1 + 2\necho x\n"), "\n3");
    assert_eq!(run_script("echo 'a' .. 'b' 1.5\n"), "\nab 1.5");
}

#[test]
fn statement_level_calls_suspend_and_resume() {
    let mut host = MemoryHost::new();
    host.suspend_after = Some(1);
    let mut rt = Runtime::new(ScriptState::new());
    rt.push_script(
        "s.vim",
        b"function! F()\n  set ts=4\n  echo 'after'\nendfunction\ncall F()\necho 'end'\n",
    )
    .unwrap();
    assert_eq!(rt.run(&mut host), RunState::Suspended);
    assert_eq!(host.output, "");
    host.suspend_after = None;
    assert_eq!(rt.run(&mut host), RunState::Complete);
    assert_eq!(host.output, "\nafter\nend");
}

/// `VIEM_DEBUG_SCRIPT=path cargo test --lib vimscript::tests::debug_script -- --ignored --nocapture`
#[test]
#[ignore]
fn debug_script() {
    let path = std::env::var("VIEM_DEBUG_SCRIPT").expect("VIEM_DEBUG_SCRIPT");
    let script = std::fs::read_to_string(path).unwrap();
    println!("{}", run_script(&script));
}

#[test]
fn sourced_scripts_preserve_scopes_finish_and_source_event_order() {
    let mut host = MemoryHost::new();
    host.files.insert("middle.vim".into(), br#"let s:value = 'middle'
function! s:Get()
  return s:value
endfunction
let g:Middle = function('s:Get')
source inner.vim
let s:value .= '-after'
"#.to_vec());
    host.files.insert("inner.vim".into(), br#"let s:value = 'inner'
function! s:Get()
  return s:value
endfunction
let g:Inner = function('s:Get')
finish
let s:value = 'wrong'
"#.to_vec());
    let runtime = run_with(&mut host, r#"let g:events = []
augroup SourceFixture
  autocmd!
  autocmd SourcePre *.vim call add(g:events, expand('<afile>:t') .. ':pre')
  autocmd SourcePost *.vim call add(g:events, expand('<afile>:t') .. ':post')
augroup END
source middle.vim
echo g:Middle() g:Inner()
echo g:events
"#);
    assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
    assert_eq!(host.output, "\nmiddle-after inner\n['middle.vim:pre', 'inner.vim:pre', 'inner.vim:post', 'middle.vim:post']");
}

#[test]
fn builtin_generated_container_limits_and_extreme_ranges() {
    use super::value::Blob;
    let mut rt = Runtime::new(ScriptState::new());
    let mut host = MemoryHost::new();
    let mut call = |name: &str, args| super::builtins::call(&mut rt, &mut host, name, args).unwrap();
    for value in [Value::Blob(Blob::new(vec![0])), Value::str("x"), Value::list(vec![1.into()])] {
        assert!(matches!(call("repeat", vec![value, i64::MAX.into()]), Err(Fail::Interrupt(_))));
    }
    assert!(matches!(call("range", vec![i64::MIN.into(), i64::MAX.into()]), Err(Fail::Interrupt(_))));
    assert!(matches!(call("range", vec![i64::MIN.into()]), Err(Fail::Error(_))));
    assert!(matches!(call("range", vec![1_000_001.into()]), Err(Fail::Interrupt(_))));
    for (args, expected) in [
        (vec![i64::MAX.into(), i64::MAX.into()], format!("[{}]", i64::MAX)),
        (vec![i64::MIN.into(), i64::MIN.into(), (-1).into()], format!("[{}]", i64::MIN)),
        (vec![0.into(), i64::MIN.into(), i64::MIN.into()], format!("[0, {}]", i64::MIN)),
        (vec![0.into()], "[]".to_string()),
        (vec![1.into(), 0.into(), 2.into()], "[]".to_string()),
    ] {
        assert_eq!(String::from_utf8(call("range", args).unwrap().string_bytes()).unwrap(), expected);
    }
}

#[test]
fn flatten_limits_reject_cycles_and_depth_without_mutation() {
    use super::value::List;
    let mut rt = Runtime::new(ScriptState::new());
    let mut host = MemoryHost::new();
    let cyclic = List::new(vec![1.into()]);
    cyclic.push(Value::List(cyclic.clone()));
    assert!(matches!(super::builtins::call(&mut rt, &mut host, "flatten", vec![Value::List(cyclic.clone())]).unwrap(), Err(Fail::Interrupt(_))));
    assert_eq!(cyclic.len(), 2);
    assert!(matches!(cyclic.get(1), Some(Value::List(child)) if child.ptr_eq(&cyclic)));
    // Break the intentional cycle so the test does not leak its Arc.
    cyclic.data().items.clear();

    let mut levels = vec![List::new(vec![7.into()])];
    for _ in 0..300 {
        levels.push(List::new(vec![Value::List(levels.last().unwrap().clone())]));
    }
    assert!(matches!(super::builtins::call(&mut rt, &mut host, "flattennew", vec![Value::List(levels.last().unwrap().clone())]).unwrap(), Err(Fail::Interrupt(_))));
    for list in &levels {
        assert_eq!(list.len(), 1);
        list.data().items.clear();
    }

    let child = Value::list(vec![1.into(), 2.into()]);
    let shared = Value::list(vec![child.clone(), child]);
    let flat = super::builtins::call(&mut rt, &mut host, "flattennew", vec![shared]).unwrap().unwrap();
    assert_eq!(flat.string_bytes(), b"[1, 2, 1, 2]");
    rt.set_fuel(1);
    assert!(matches!(super::builtins::call(&mut rt, &mut host, "flatten", vec![flat]).unwrap(), Err(Fail::Interrupt(_))));
}
