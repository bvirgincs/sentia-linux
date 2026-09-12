use sentia_tools::{
    command_exists, command_history, directory_contents, file_exists, os_information,
    package_owns_file, process_details, service_list, CommandExistsInput, CommandHistoryEntry,
    CommandHistoryInput, DirectoryContentsInput, FilePathInput, OsInformationInput,
    PackageOwnsFileInput, ProcessDetailsInput, ServiceListInput, ToolError,
};

#[test]
fn host_probe_basic_read_only_tools() {
    let command = command_exists(CommandExistsInput {
        command: "ls".to_string(),
    })
    .expect("command_exists should work");
    assert_eq!(command.tool_name, "command_exists");
    assert!(command.data["exists"].as_bool().is_some());

    let history = command_history(CommandHistoryInput {
        entries: vec![
            CommandHistoryEntry {
                command: "ls -la".to_string(),
                cwd: Some("/home/ubuntu".into()),
                exit_code: Some(0),
                timestamp: Some("2026-09-12T00:00:00Z".to_string()),
            },
            CommandHistoryEntry {
                command: "false".to_string(),
                cwd: Some("/home/ubuntu".into()),
                exit_code: Some(1),
                timestamp: Some("2026-09-12T00:01:00Z".to_string()),
            },
        ],
        query: Some("ls".to_string()),
        limit: Some(10),
        failures_only: false,
    })
    .expect("command_history should process explicit entries");
    let entry_count = history
        .data["entries"]
        .as_array()
        .map(Vec::len)
        .expect("entries must be an array");
    assert!((1..=10).contains(&entry_count));

    let exists = file_exists(FilePathInput {
        path: "/etc/passwd".into(),
        allow_sensitive: false,
    })
    .expect("file_exists should run");
    assert_eq!(exists.data["exists"].as_bool(), Some(true));

    let listing = directory_contents(DirectoryContentsInput {
        path: "/etc".into(),
        limit: Some(5),
        include_hidden: false,
        allow_sensitive: false,
    })
    .expect("directory_contents should run");
    assert!(listing.data["entries"].as_array().is_some());

    let details = process_details(ProcessDetailsInput {
        pid: std::process::id() as i32,
        include_cmdline: false,
    })
    .expect("process_details should run for current process");
    assert_eq!(
        details.data["process"]["pid"].as_i64(),
        Some(std::process::id() as i64)
    );

    let os = os_information(OsInformationInput).expect("os_information should work");
    assert_eq!(os.tool_name, "os_information");
}

#[test]
fn host_probe_package_and_service_error_shapes() {
    let package = package_owns_file(PackageOwnsFileInput {
        path: "/bin/ls".into(),
        allow_sensitive: false,
    });
    match package {
        Ok(result) => assert_eq!(result.tool_name, "package_owns_file"),
        Err(ToolError::NotFound { .. }) => {}
        Err(err) => panic!("unexpected package_owns_file error: {err}"),
    }

    let service = service_list(ServiceListInput {
        limit: Some(3),
        include_inactive: true,
    });
    match service {
        Ok(result) => assert_eq!(result.tool_name, "service_list"),
        Err(ToolError::Unavailable { .. } | ToolError::PermissionDenied { .. }) => {}
        Err(err) => panic!("unexpected service_list error: {err}"),
    }
}
