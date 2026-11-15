//! Function/runtime regressions checked against Vim 9.2.
use super::tests::{run_script, run_with, MemoryHost};

fn checked(script: &str) -> String {
    let mut host = MemoryHost::new();
    let runtime = run_with(&mut host, script);
    assert!(runtime.errors.is_empty(), "{:?}: {}", runtime.errors, host.output);
    host.output
}

#[test]
fn quoted_legacy_return_values_are_expressions() {
    assert_eq!(checked("function! Greeting()\n  return \"hello\"\nendfunction\necho Greeting()\n"), "\nhello");
}

#[test]
fn funcref_keeps_the_original_definition_while_function_resolves_the_name() {
    let script = "function! Foo()\n  let g:called = 1\n  return 1\nendfunction\nlet Saved = funcref('Foo')\nlet Dynamic = function('Foo')\nfunction! Foo()\n  let g:called = 2\n  return 2\nendfunction\necho Saved() Dynamic()\ncall Saved()\necho g:called\ndelfunction Foo\necho Saved()\n";
    assert_eq!(checked(script), "\n1 2\n1\n1");
}

#[test]
fn funcref_captures_existing_late_bound_partials() {
    let script = "function! Plain(n)\n  return a:n + 1\nendfunction\nlet Late = function('Plain', [10])\nlet Saved = funcref(Late)\nfunction! Plain(n)\n  return a:n + 2\nendfunction\necho Late() Saved()\n";
    assert_eq!(checked(script), "\n12 11");
}

#[test]
fn saved_dictionary_methods_retain_dot_and_index_receivers() {
    let script = "let d = {'key': 'receiver'}\nfunction! d.Get() dict\n  return self.key\nendfunction\nlet Saved = d.Get\nlet Indexed = d['Get']\necho Saved() Indexed()\n";
    assert_eq!(checked(script), "\nreceiver receiver");
}

#[test]
fn dictionary_member_access_binds_only_dictionary_functions() {
    let script = "function! Plain()\n  return 5\nendfunction\nlet d = {'Len': function('len'), 'Plain': function('Plain')}\nlet Len = d.Len\nlet SavedPlain = d.Plain\necho get(Len, 'dict') get(SavedPlain, 'dict') Len('abc') SavedPlain()\n";
    assert_eq!(checked(script), "\n0 0 3 5");
}

#[test]
fn deleting_a_dictionary_alias_leaves_the_global_function_callable() {
    let script = "function! Foo()\n  return 'alive'\nendfunction\nlet d = {'Alias': function('Foo')}\ndelfunction d.Alias\necho has_key(d, 'Alias') Foo()\n";
    assert_eq!(checked(script), "\n0 alive");
}

#[test]
fn extra_arguments_list_is_fixed_but_its_argument_values_remain_mutable() {
    let script = "function! Extra(...) abort\n  call add(a:000[0], 'ok')\n  let Rest = a:000\n  unlockvar! Rest\n  try\n    call add(Rest, 'bad')\n  catch /E742:/\n    echo 'fixed' a:0 len(a:000)\n  endtry\nendfunction\nlet input = []\ncall Extra(input)\necho input\n";
    assert_eq!(checked(script), "\nfixed 1 1\n['ok']");
}

#[test]
fn nested_indexof_callbacks_restore_outer_callback_variables() {
    let script = "function! Nested(value)\n  let before = v:val\n  call indexof(['inner'], 'v:val ==# \"inner\"')\n  return [before, v:val]\nendfunction\necho map(['outer'], 'Nested(v:val)')\n";
    assert_eq!(checked(script), "\n[['outer', 'outer']]");
}

#[test]
fn special_variable_assignment_preserves_declared_types_and_rejects_atomically() {
    let script = "let v:errmsg = 123\necho type(v:errmsg) v:errmsg\nlet original = v:errors\ntry\n  let v:errors = 'oops'\ncatch /E963:/\n  echo type(v:errors) (v:errors is original)\nendtry\n";
    assert_eq!(checked(script), "\n1 123\n3 1");
}

#[test]
fn invalid_vim9_comparisons_fail_compilation_before_body_effects() {
    let script = "vim9script\ng:ran = false\ndef Invalid(): bool\n  g:ran = true\n  return '3' == 3\nenddef\ntry\n  Invalid()\ncatch /E1072:/\n  echo g:ran\nendtry\necho 3 == 3.0\n";
    assert_eq!(checked(script), "\nfalse\ntrue");
}

#[test]
fn importing_an_already_sourced_vim9_module_reuses_its_initialized_namespace() {
    let absolute = std::env::current_dir().unwrap().join("dep.vim").to_string_lossy().replace('\\', "/");
    for path in ["./dep.vim".to_owned(), absolute] {
        let mut host = MemoryHost::new();
        host.files.insert(path.clone(), b"vim9script\ng:initializations += 1\nexport var saved = g:initializations\nvar hidden = 0\n".to_vec());
        let command_path = path.replace(' ', "\\ ");
        let runtime = run_with(&mut host, &format!("vim9script\ng:initializations = 0\nsource {command_path}\nimport './dep.vim' as dep\necho g:initializations dep.saved\ntry\n  echo dep.hidden\ncatch /E1049:/\n  echo 'private'\nendtry\n"));
        assert!(runtime.errors.is_empty(), "{:?}: {}", runtime.errors, host.output);
        assert_eq!(host.output, "\n1 1\nprivate");
    }
}

#[test]
fn enum_identity_is_available_to_field_initializers() {
    let script = "vim9script\nenum Flag\n  Alpha, Beta\n  var label: string = this.name\n  var index: number = this.ordinal\nendenum\necho Flag.Alpha.label Flag.Beta.label Flag.Alpha.index Flag.Beta.index\n";
    assert_eq!(checked(script), "\nAlpha Beta 0 1");
}

#[test]
fn assert_fails_checks_first_and_last_error_patterns() {
    let script = "function! F() abort\n  call Missing()\nendfunction\necho assert_fails('call F()', ['E117:', 'never-printed']) len(v:errors)\nlet v:errors = []\necho assert_fails('call F()', ['^E117:', 'Missing']) len(v:errors)\n";
    assert_eq!(checked(script), "\n1 1\n0 0");
}

#[test]
fn failed_indexof_callbacks_restore_the_outer_value_before_unwinding() {
    let output = run_script("function! Nested() abort\n  try\n    call indexof(['inner'], 'Missing()')\n  catch /E117:/\n    return v:val\n  endtry\nendfunction\necho map(['outer'], 'Nested()')\n");
    assert_eq!(output, "\n['outer']");
}
