// SPDX-License-Identifier: Apache-2.0
//! Real D-Bus transport, controlled logind/polkit peers; no host system-bus use.
use sentia_privileged::broker::Prepared;
use sentia_privileged::{service::SystemService, BUS_NAME, OBJECT_PATH};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

struct PrivateBus(Child);

impl std::ops::Deref for PrivateBus {
    type Target = Child;
    fn deref(&self) -> &Child { &self.0 }
}

impl std::ops::DerefMut for PrivateBus {
    fn deref_mut(&mut self) -> &mut Child { &mut self.0 }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Manager;

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, _pid: u32) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/test").unwrap()
    }
}

struct Session {
    active: Arc<AtomicBool>,
    remote: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl Session {
    #[zbus(property)]
    fn active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn remote(&self) -> bool {
        self.remote.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn class(&self) -> &str {
        "user"
    }
    #[zbus(property)]
    fn user(&self) -> (u32, OwnedObjectPath) {
        (
            unsafe { libc::getuid() },
            OwnedObjectPath::try_from("/org/freedesktop/login1/user/test").unwrap(),
        )
    }
}

type AuthCalls = Arc<Mutex<Vec<(String, String, u32)>>>;

struct Authority {
    calls: AuthCalls,
    authorized: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    deactivate: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl Authority {
    fn check_authorization(
        &self,
        subject: (String, HashMap<String, OwnedValue>),
        action: String,
        details: HashMap<String, String>,
        flags: u32,
        cancellation: String,
    ) -> (bool, bool, HashMap<String, String>) {
        assert_eq!(subject.0, "system-bus-name");
        assert_eq!(subject.1.len(), 1);
        assert!(details.is_empty());
        assert!(cancellation.is_empty());
        let name = <&str>::try_from(subject.1.get("name").unwrap()).unwrap();
        self.calls.lock().unwrap().push((name.into(), action, flags));
        if self.deactivate.load(Ordering::SeqCst) {
            self.active.store(false, Ordering::SeqCst);
        }
        (self.authorized.load(Ordering::SeqCst), false, HashMap::new())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dbus_caller_and_authorization_gates_with_owned_child() {
    let mut daemon = Command::new("/usr/bin/dbus-daemon")
        .args([
            "--session",
            "--nofork",
            "--nopidfile",
            "--print-address=1",
            &format!(
                "--address=unix:abstract=sentia-privileged-test-{}",
                std::process::id()
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("dbus-daemon is required for isolated transport validation");
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let _daemon = PrivateBus(daemon);
    let address = address.trim();
    assert!(address.starts_with("unix:abstract=sentia-privileged-test-"));
    let active = Arc::new(AtomicBool::new(true));
    let remote = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let authorized = Arc::new(AtomicBool::new(false));
    let deactivate = Arc::new(AtomicBool::new(false));
    let server = zbus::connection::Builder::address(address)
        .unwrap()
        .serve_at("/org/freedesktop/login1", Manager)
        .unwrap()
        .serve_at(
            "/org/freedesktop/login1/session/test",
            Session {
                active: active.clone(),
                remote: remote.clone(),
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/PolicyKit1/Authority",
            Authority {
                calls: calls.clone(),
                authorized: authorized.clone(),
                active: active.clone(),
                deactivate: deactivate.clone(),
            },
        )
        .unwrap()
        .serve_at(OBJECT_PATH, SystemService::default())
        .unwrap()
        .build()
        .await
        .unwrap();
    for name in [BUS_NAME, "org.freedesktop.login1", "org.freedesktop.PolicyKit1"] {
        server.request_name(name).await.unwrap();
    }
    let client = zbus::connection::Builder::address(address).unwrap().build().await.unwrap();
    let proxy = zbus::Proxy::new(&client, BUS_NAME, OBJECT_PATH, BUS_NAME).await.unwrap();
    let mut target = PrivateBus(Command::new("/usr/bin/sleep")
        .arg("60")
        .env_clear()
        .spawn()
        .unwrap());
    let request = serde_json::json!({
        "version":1,
        "operation":{"operation":"process_kill", "pid":target.id()}
    })
    .to_string();
    let json: String = proxy.call("Prepare", &(&request,)).await.unwrap();
    let prepared: Prepared = serde_json::from_str(&json).unwrap();
    assert_eq!(prepared.plan.caller.unique_name, client.unique_name().unwrap().as_str());
    assert_eq!(prepared.plan.caller.uid, unsafe { libc::getuid() });
    assert_eq!(prepared.plan.caller.pid, std::process::id());
    let other = zbus::connection::Builder::address(address).unwrap().build().await.unwrap();
    let other_proxy = zbus::Proxy::new(&other, BUS_NAME, OBJECT_PATH, BUS_NAME).await.unwrap();
    let denied: zbus::Result<String> = other_proxy
        .call("Apply", &(&prepared.plan.id, &prepared.digest))
        .await;
    assert!(denied.unwrap_err().to_string().contains("caller_changed"));
    let denied: zbus::Result<String> = proxy
        .call("Apply", &(&prepared.plan.id, &prepared.digest))
        .await;
    assert!(denied.unwrap_err().to_string().contains("authorization_denied"));
    assert!(target.try_wait().unwrap().is_none());
    let replay: zbus::Result<String> = proxy
        .call("Apply", &(&prepared.plan.id, &prepared.digest))
        .await;
    assert!(replay.unwrap_err().to_string().contains("unknown_or_used_plan"));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[(
            client.unique_name().unwrap().to_string(),
            "org.sentia.system.process-terminate".into(),
            1
        )]
    );
    authorized.store(true, Ordering::SeqCst);
    deactivate.store(true, Ordering::SeqCst);
    let json: String = proxy.call("Prepare", &(&request,)).await.unwrap();
    let prepared: Prepared = serde_json::from_str(&json).unwrap();
    let denied: zbus::Result<String> = proxy
        .call("Apply", &(&prepared.plan.id, &prepared.digest))
        .await;
    assert!(denied.unwrap_err().to_string().contains("inactive_or_remote_session"));
    assert!(target.try_wait().unwrap().is_none());
    active.store(true, Ordering::SeqCst);
    deactivate.store(false, Ordering::SeqCst);
    let json: String = proxy.call("Prepare", &(&request,)).await.unwrap();
    let prepared: Prepared = serde_json::from_str(&json).unwrap();
    let applied: String = proxy
        .call("Apply", &(&prepared.plan.id, &prepared.digest))
        .await.unwrap();
    let applied: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert_eq!(applied["status"], "terminated");
    assert_eq!(applied["signal"], "SIGTERM");
    assert!(!target.wait().unwrap().success());
    active.store(false, Ordering::SeqCst);
    let denied: zbus::Result<String> = proxy.call("Prepare", &(&request,)).await;
    assert!(denied.unwrap_err().to_string().contains("inactive_or_remote_session"));
    active.store(true, Ordering::SeqCst);
    remote.store(true, Ordering::SeqCst);
    let denied: zbus::Result<String> = proxy.call("Prepare", &(&request,)).await;
    assert!(denied.unwrap_err().to_string().contains("inactive_or_remote_session"));
    let injected = serde_json::json!({
        "version":1, "approved":true,
        "operation":{"operation":"process_kill","pid":target.id()}
    }).to_string();
    let denied: zbus::Result<String> = proxy.call("Prepare", &(&injected,)).await;
    assert!(denied.unwrap_err().to_string().contains("invalid_request"));
}
