use evim_core::command::ex::{parse_ex, ExAction};
use evim_core::command::ex_execute::{
    execute_ex, ExExecutionContext, ExExecutionState, ExFileRequest, ExFrontendRequest,
    ExRegisterReader, ExRegisterValue,
};
use evim_core::document::Document;
struct Empty;
impl ExRegisterReader for Empty {
    fn read(&self, _: Option<char>) -> Option<ExRegisterValue> {
        None
    }
}
fn execute(document: &mut Document, command: &str) -> Vec<ExFrontendRequest> {
    execute_ex(
        document,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &parse_ex(command).unwrap(),
        &Empty,
    )
    .unwrap()
    .frontend_requests
}
#[test]
fn uppercase_edit_has_explicit_new_window_effect_and_keeps_source() {
    let mut doc = Document::new("saved");
    assert!(
        matches!(parse_ex(":E file with spaces.txt").unwrap().action, ExAction::EditNewWindow { path: Some(ref p) } if p=="file with spaces.txt")
    );
    assert_eq!(
        execute(&mut doc, ":E foo.txt"),
        vec![ExFrontendRequest::File(ExFileRequest::EditNewWindow {
            path: Some("foo.txt".into())
        })]
    );
    assert_eq!(
        execute(&mut doc, ":e! foo.txt"),
        vec![ExFrontendRequest::File(ExFileRequest::Edit {
            path: Some("foo.txt".into()),
            force: true
        })]
    );
    assert_eq!(doc.text(), "saved");
    assert!(!doc.is_dirty());
    assert!(parse_ex(":2E foo").is_err());
}
#[test]
fn pwd_cd_and_update_are_typed_and_update_only_writes_dirty_source() {
    let mut doc = Document::new("saved");
    assert_eq!(
        execute(&mut doc, ":pwd"),
        vec![ExFrontendRequest::File(
            ExFileRequest::PrintWorkingDirectory
        )]
    );
    assert_eq!(
        execute(&mut doc, ":chdir ~/words"),
        vec![ExFrontendRequest::File(ExFileRequest::ChangeDirectory {
            path: Some("~/words".into())
        })]
    );
    assert!(execute(&mut doc, ":up").is_empty());
    doc.replace(0..0, "new ").unwrap();
    assert_eq!(
        execute(&mut doc, ":up"),
        vec![ExFrontendRequest::File(ExFileRequest::Write {
            path: None,
            force: false,
            range: None
        })]
    );
    doc.set_read_only(true);
    assert!(execute_ex(
        &mut doc,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &parse_ex(":up").unwrap(),
        &Empty
    )
    .is_err());
    assert_eq!(
        execute(&mut doc, ":up!"),
        vec![ExFrontendRequest::File(ExFileRequest::Write {
            path: None,
            force: true,
            range: None
        })]
    );
    assert!(parse_ex(":1pwd").is_err());
    assert!(parse_ex(":pwd argument").is_err());
    assert!(parse_ex(":cd!").is_err());
    assert!(parse_ex(":up file").is_err());
}
#[test]
fn substitute_filename_diagnostic_preserves_substitute_and_save_grammar() {
    let error = parse_ex(":s bar.txt").unwrap_err().to_string();
    assert!(error.contains(":w <file>"));
    assert!(error.contains(":saveas <file>"));
    assert!(matches!(
        parse_ex(":s/bar/baz/").unwrap().action,
        ExAction::Substitute(_)
    ));
    assert!(matches!(
        parse_ex(":w bar.txt").unwrap().action,
        ExAction::Write { path: Some(_) }
    ));
}
