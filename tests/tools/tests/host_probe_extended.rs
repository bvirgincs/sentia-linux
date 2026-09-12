use sentia_tools::{
    command_help, command_lookup, disk_usage, file_permissions, file_type, hardware_information,
    journal_errors, journal_recent, kernel_logs, listening_ports, network_interfaces,
    network_status, process_list, service_status, CommandHelpInput, CommandLookupInput,
    DiskUsageInput, FilePathInput, HardwareInformationInput, JournalErrorsInput,
    JournalRecentInput, KernelLogsInput, ListeningPortsInput, NetworkInterfacesInput,
    NetworkStatusInput, ProcessListInput, ServiceStatusInput, ToolError,
};

#[test]
fn host_probe_extended_read_only_coverage() {
    let lookup = command_lookup(CommandLookupInput {
        command: "ls".to_string(),
    })
    .expect("lookup ls");
    assert_eq!(lookup.tool_name, "command_lookup");

    let help = command_help(CommandHelpInput {
        command: "ls".to_string(),
        section: None,
        max_chars: Some(2_000),
    });
    match help {
        Ok(result) => assert_eq!(result.tool_name, "command_help"),
        Err(ToolError::NotFound { .. } | ToolError::Unavailable { .. }) => {}
        Err(err) => panic!("unexpected command_help error: {err}"),
    }

    let typ = file_type(FilePathInput {
        path: "/etc/passwd".into(),
        allow_sensitive: false,
    })
    .expect("file_type");
    assert_eq!(typ.tool_name, "file_type");

    let perms = file_permissions(FilePathInput {
        path: "/etc/passwd".into(),
        allow_sensitive: false,
    })
    .expect("file_permissions");
    assert_eq!(perms.tool_name, "file_permissions");

    let usage = disk_usage(DiskUsageInput {
        path: "/".into(),
        allow_sensitive: false,
    })
    .expect("disk_usage");
    assert_eq!(usage.tool_name, "disk_usage");

    let net_status = network_status(NetworkStatusInput).expect("network_status");
    assert_eq!(net_status.tool_name, "network_status");

    let net_ifaces = network_interfaces(NetworkInterfacesInput).expect("network_interfaces");
    assert_eq!(net_ifaces.tool_name, "network_interfaces");

    let ports = listening_ports(ListeningPortsInput {
        limit: Some(50),
        include_udp: true,
    })
    .expect("listening_ports");
    assert_eq!(ports.tool_name, "listening_ports");

    let proc_list = process_list(ProcessListInput {
        limit: Some(20),
        include_cmdline: false,
    })
    .expect("process_list");
    assert_eq!(proc_list.tool_name, "process_list");

    let hw = hardware_information(HardwareInformationInput).expect("hardware_information");
    assert_eq!(hw.tool_name, "hardware_information");
}

#[test]
fn host_probe_log_and_service_permissions_are_explicit() {
    let recent = journal_recent(JournalRecentInput {
        lines: Some(5),
        unit: None,
    });
    assert!(matches!(
        recent,
        Ok(_) | Err(ToolError::Unavailable { .. } | ToolError::PermissionDenied { .. })
    ));

    let errors = journal_errors(JournalErrorsInput { lines: Some(5) });
    assert!(matches!(
        errors,
        Ok(_) | Err(ToolError::Unavailable { .. } | ToolError::PermissionDenied { .. })
    ));

    let kernel = kernel_logs(KernelLogsInput { lines: Some(5) });
    assert!(matches!(
        kernel,
        Ok(_) | Err(ToolError::Unavailable { .. } | ToolError::PermissionDenied { .. })
    ));

    let status = service_status(ServiceStatusInput {
        service: "ssh".to_string(),
    });
    assert!(matches!(
        status,
        Ok(_)
            | Err(
                ToolError::Unavailable { .. }
                    | ToolError::PermissionDenied { .. }
                    | ToolError::NotFound { .. }
            )
    ));
}
