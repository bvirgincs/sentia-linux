use sentia_tools::{
    command_history, file_exists, CommandHistoryInput, FilePathInput, ToolError,
};

#[test]
fn sensitive_path_requires_opt_in() {
    let result = file_exists(FilePathInput {
        path: "/etc/shadow".into(),
        allow_sensitive: false,
    });

    assert!(matches!(result, Err(ToolError::PermissionDenied { .. })));
}

#[test]
fn command_history_requires_explicit_entries() {
    let result = command_history(CommandHistoryInput {
        entries: vec![],
        query: None,
        limit: None,
        failures_only: false,
    });

    assert!(matches!(result, Err(ToolError::Unavailable { .. })));
}
