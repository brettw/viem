use viem_core::command::ex::{parse_ex, ExAction};
use viem_core::command::ex_execute::{
    execute_ex, ExExecutionContext, ExExecutionState, ExFileRequest, ExFrontendRequest,
    ExRegisterReader, ExRegisterValue,
};
use viem_core::document::Document;
struct Empty;
impl ExRegisterReader for Empty {
    fn read(&self, _: Option<char>) -> Option<ExRegisterValue> {
        None
    }
}
#[test]
fn checktime_is_a_read_only_typed_host_request_without_counts_or_arguments() {
    let mut document = Document::new("unsaved text");
    document.insert(0, "new ").unwrap();
    let history = document.history_status();
    let source = document.source_bytes();
    let revision = document.revision();
    for spelling in [":checktime", ":checkt"] {
        let command = parse_ex(spelling).unwrap();
        assert_eq!(command.action, ExAction::CheckTime);
        let result = execute_ex(
            &mut document,
            &mut ExExecutionState::default(),
            &ExExecutionContext::default(),
            &command,
            &Empty,
        )
        .unwrap();
        assert_eq!(
            result.frontend_requests,
            [ExFrontendRequest::File(ExFileRequest::CheckTime)]
        );
        assert_eq!(document.history_status(), history);
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
    }
    for invalid in [
        ":2checktime",
        ":checktime!",
        ":checktime foo",
        ":%checktime",
    ] {
        assert!(parse_ex(invalid).is_err(), "{invalid}");
    }
}
