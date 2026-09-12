// SPDX-License-Identifier: Apache-2.0
use sentia_privileged::broker::{revalidate, Caller, PlanStore};
use sentia_privileged::executor::{apt_request, parse_start_ticks, service_arguments, trusted_path};
use sentia_privileged::{parse_request, validate_package, validate_unit, Operation, ServiceAction};
use serde_json::json;

fn caller() -> Caller {
    Caller {
        unique_name: ":1.42".into(),
        uid: 1000,
        pid: 4000,
        process_start: 12345,
        session: "/org/freedesktop/login1/session/_31".into(),
    }
}

fn operation() -> Operation {
    Operation::Service {
        action: ServiceAction::Restart,
        unit: "ssh.service".into(),
    }
}

#[test]
fn typed_request_rejects_injected_identity_approval_and_shell() {
    for request in [
        json!({"version":1,"operation":{"operation":"shell","command":"id"}}),
        json!({"version":1,"operation":{"operation":"process_kill","pid":2},"approved":true}),
        json!({"version":1,"operation":{"operation":"process_kill","pid":2},"uid":0}),
        json!({"version":1,"operation":{"operation":"process_kill","pid":2,"approved":true}}),
        json!({"version":1,"operation":{"operation":"service","action":"reload","unit":"ssh.service"}}),
        json!({"version":1,"operation":{"operation":"apt_install","packages":["curl"],"environment":{"LD_PRELOAD":"bad"}}}),
        json!({"version":2,"operation":{"operation":"apt_update"}}),
    ] {
        assert!(parse_request(&request.to_string()).is_err(), "{request}");
    }
    assert!(parse_request(&"x".repeat(16_385)).is_err());
    assert!(parse_request(r#"{"version":1,"operation":{"operation":"process_kill","pid":2}}"#).is_ok());
}

#[test]
fn unit_names_are_bounded_literal_services_not_options_paths_or_globs() {
    for unit in [
        "",
        ".service",
        "../ssh.service",
        "/etc/systemd/ssh.service",
        "--help.service",
        "ssh.socket",
        "ssh.target",
        "ssh@user.service",
        "ssh*.service",
        "ssh?.service",
        "ssh\\x20.service",
        "ssh\n.service",
        "ssh;id.service",
        "ssh\x00.service",
        "ssh..service",
        "śsh.service",
    ] {
        assert!(validate_unit(unit).is_err(), "{unit:?}");
    }
    assert!(validate_unit(&format!("{}.service", "x".repeat(247))).is_ok());
    assert!(validate_unit(&format!("{}.service", "x".repeat(248))).is_err());
    for unit in ["ssh.service", "dbus-org.freedesktop.login1.service", "a_b-2.service"] {
        assert!(validate_unit(unit).is_ok());
    }
}

#[test]
fn service_argv_is_fixed_and_option_terminated() {
    for action in [
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
        ServiceAction::Enable,
    ] {
        let args = service_arguments(&action, "ssh.service").unwrap();
        assert_eq!(
            args,
            ["--no-ask-password", "--no-pager", action.argument(), "--", "ssh.service"]
        );
    }
    assert!(service_arguments(&ServiceAction::Start, "--root=/home/x").is_err());
}

#[test]
fn process_kill_cannot_target_pid_one_groups_or_overflow() {
    for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert!(Operation::ProcessKill { pid }.validate().is_err());
    }
    for request in [
        r#"{"version":1,"operation":{"operation":"process_kill","pid":-1}}"#,
        r#"{"version":1,"operation":{"operation":"process_kill","pid":4294967296}}"#,
        r#"{"version":1,"operation":{"operation":"process_kill","pid":22,"signal":9}}"#,
        r#"{"version":1,"operation":{"operation":"process_kill","pid":"22"}}"#,
    ] {
        assert!(parse_request(request).is_err());
    }
}

#[test]
fn package_identifiers_do_not_accept_options_versions_paths_or_shell() {
    for package in ["-y", "--allow-unauthenticated", "/bin/sh", "x", "curl=1", "curl:amd64", "curl*", "curl;id", "Curl", "a b"] {
        assert!(validate_package(package).is_err(), "{package}");
    }
    for package in ["curl", "libstdc++6", "python3.13", "7zip"] {
        assert!(validate_package(package).is_ok());
    }
    assert!(validate_package(&"a".repeat(129)).is_err());
    assert!(Operation::AptInstall { packages: vec![] }.validate().is_err());
    assert!(Operation::AptInstall { packages: vec!["curl".into(), "curl".into()] }.validate().is_err());
    assert!(Operation::AptInstall { packages: (0..65).map(|i| format!("pkg{i}")).collect() }.validate().is_err());
}

#[test]
fn plans_are_nonce_bound_digest_bound_and_single_use() {
    let mut store = PlanStore::default();
    let first = store.prepare(caller(), operation(), json!({"generation":1}), 100).unwrap();
    let second = store.prepare(caller(), operation(), json!({"generation":1}), 100).unwrap();
    assert_ne!(first.plan.id, second.plan.id);
    assert_ne!(first.digest, second.digest);
    assert_eq!(first.plan.expires_at, 220);
    assert_eq!(first.plan.action_id, "org.sentia.system.service-restart");
    assert!(store.consume(&first.plan.id, &first.digest, &caller(), 101).is_ok());
    assert_eq!(store.consume(&first.plan.id, &first.digest, &caller(), 101).unwrap_err().0, "unknown_or_used_plan");
    assert!(store.consume(&second.plan.id, &"0".repeat(64), &caller(), 101).is_err());
    assert!(store.consume(&second.plan.id, &second.digest, &caller(), 101).is_err());
}

#[test]
fn every_caller_identity_component_is_bound() {
    let mut impostors = vec![caller(); 5];
    impostors[0].unique_name = ":1.43".into();
    impostors[1].uid = 0;
    impostors[2].pid += 1;
    impostors[3].process_start += 1;
    impostors[4].session = "/org/freedesktop/login1/session/_32".into();
    let mut store = PlanStore::default();
    let prepared = store.prepare(caller(), operation(), json!({}), 100).unwrap();
    for impostor in impostors {
        assert_eq!(store.consume(&prepared.plan.id, &prepared.digest, &impostor, 101).unwrap_err().0, "caller_changed");
    }
    assert!(store.consume(&prepared.plan.id, &prepared.digest, &caller(), 101).is_ok());
}

#[test]
fn expiry_inclusive_boundary_and_backward_clock_fail_closed() {
    for time in [99, 220, u64::MAX] {
        let mut store = PlanStore::default();
        let prepared = store.prepare(caller(), operation(), json!({}), 100).unwrap();
        assert_eq!(store.consume(&prepared.plan.id, &prepared.digest, &caller(), time).unwrap_err().0, "plan_expired");
    }
    assert!(PlanStore::default().prepare(caller(), operation(), json!({}), u64::MAX).is_err());
}

#[test]
fn state_and_plan_are_revalidated_after_authorization() {
    let original_state = json!({"pid":2,"start_ticks":123,"uid":1000});
    let mut store = PlanStore::default();
    let prepared = store.prepare(caller(), Operation::ProcessKill { pid: 2 }, original_state.clone(), 100).unwrap();
    assert!(revalidate(&prepared.plan, &caller(), &original_state, 101).is_ok());
    for changed in [
        json!({"pid":2,"start_ticks":124,"uid":1000}),
        json!({"pid":2,"start_ticks":123,"uid":0}),
        json!({"packages":["extra-unapproved-dependency"]}),
    ] {
        assert_eq!(revalidate(&prepared.plan, &caller(), &changed, 101).unwrap_err().0, "state_changed_prepare_again");
    }
    let mut changed = prepared.plan.clone();
    changed.operation = Operation::ProcessKill { pid: 3 };
    assert_eq!(revalidate(&changed, &caller(), &original_state, 101).unwrap_err().0, "plan_changed");
    assert!(revalidate(&prepared.plan, &caller(), &original_state, 220).is_err());
}

#[test]
fn pending_plans_are_bounded_and_expired_capacity_reclaimed() {
    let mut store = PlanStore::default();
    for _ in 0..8 {
        store.prepare(caller(), operation(), json!({}), 100).unwrap();
    }
    assert_eq!(store.prepare(caller(), operation(), json!({}), 100).unwrap_err().0, "too_many_pending_plans");
    assert!(store.prepare(caller(), operation(), json!({}), 220).is_ok());
}

#[test]
fn proc_stat_parser_handles_parentheses_and_rejects_malformed_data() {
    let mut fields = vec!["0"; 20];
    fields[0] = "S";
    fields[19] = "1234567";
    let stat = format!("45 (tricky ) process (name)) {}", fields.join(" "));
    assert_eq!(parse_start_ticks(&stat).unwrap(), 1234567);
    assert!(parse_start_ticks("1 (broken").is_err());
    assert!(parse_start_ticks("1 (name) R 1").is_err());
}

#[test]
fn shipped_policy_is_non_cached_and_does_not_grant_by_rule() {
    let policy = include_str!("../../config/polkit/org.sentia.system.policy");
    assert_eq!(policy.matches("<action id=").count(), 9);
    assert_eq!(policy.matches("<allow_active>auth_admin</allow_active>").count(), 9);
    assert!(!policy.contains("auth_admin_keep"));
    assert_eq!(policy.matches("<allow_any>no</allow_any>").count(), 9);
    let rules = include_str!("../../config/polkit/49-sentia-local.rules");
    assert!(!rules.contains("Result.YES"));
    assert!(rules.contains("!subject.local || !subject.active"));
    let dbus = include_str!("../../config/dbus/org.sentia.System1.conf");
    assert!(!dbus.contains("own=\"*\""));
    assert!(!dbus.contains("send_destination=\"*\""));
}

#[test]
fn apt_worker_ipc_binds_native_digest_and_never_approves_dangerous_exceptions() {
    let operation = Operation::AptInstall { packages: vec!["curl".into()] };
    let request = apt_request(&operation, None).unwrap();
    assert_eq!(request["protocol_version"], "1.0");
    assert_eq!(request["operation"], "apt_install");
    assert_eq!(request["arguments"]["mode"], "plan");
    assert!(request.get("approval").is_none());
    let prepared = PlanStore::default().prepare(
        caller(), operation.clone(),
        json!({"canonical_plan":{"requested_packages":["curl"]},"plan_digest":"a".repeat(64)}),
        1_700_000_000,
    ).unwrap();
    let execute = apt_request(&operation, Some(&prepared.plan)).unwrap();
    assert_eq!(execute["arguments"]["mode"], "execute");
    assert_eq!(execute["request_id"], prepared.plan.id);
    assert_eq!(execute["approval"]["authorization_id"], prepared.plan.id);
    assert_eq!(execute["approval"]["plan_digest"], "a".repeat(64));
    assert_eq!(execute["approval"]["expires_at"], "2023-11-14T22:15:20Z");
    for field in ["allow_source_change", "allow_essential_removal", "allow_held_change"] {
        assert_eq!(execute["approval"][field], false);
    }

    assert!(apt_request(&Operation::AptRemove { packages: vec!["curl".into()] }, Some(&prepared.plan)).is_err());
    assert!(apt_request(&Operation::ProcessKill { pid: 2 }, None).is_err());
}

#[test]
fn privileged_paths_reject_relative_components_and_symlinks() {
    use std::path::Path;
    assert!(trusted_path(Path::new("usr/bin/systemctl")).is_err());
    assert!(trusted_path(Path::new("/usr/../usr/bin/systemctl")).is_err());
    assert!(trusted_path(Path::new("/proc/self/exe")).is_err());
    assert!(trusted_path(Path::new("/usr/bin/systemctl")).is_ok());
}
