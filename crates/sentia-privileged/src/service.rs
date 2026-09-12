// SPDX-License-Identifier: Apache-2.0
use crate::broker::{revalidate, Caller, PlanStore};
use crate::executor;
use crate::{parse_request, Error, Result};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Connection, Proxy};

#[derive(Default)]
pub struct SystemService {
    store: Mutex<PlanStore>,
    operation_lock: Mutex<()>,
}

fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| Error("clock_unavailable"))
}

fn dbus_error(error: Error) -> zbus::fdo::Error {
    match error.0 {
        "authorization_denied" | "inactive_or_remote_session" | "caller_changed" => {
            zbus::fdo::Error::AccessDenied(error.0.into())
        }
        _ => zbus::fdo::Error::Failed(error.0.into()),
    }
}

pub async fn identify(connection: &Connection, unique_name: &str) -> Result<Caller> {
    if !unique_name.starts_with(':') || unique_name.len() > 255 {
        return Err(Error("invalid_bus_sender"));
    }
    let bus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .map_err(|_| Error("bus_unavailable"))?;
    let uid: u32 = bus
        .call("GetConnectionUnixUser", &(unique_name,))
        .await
        .map_err(|_| Error("caller_disconnected"))?;
    let pid: u32 = bus
        .call("GetConnectionUnixProcessID", &(unique_name,))
        .await
        .map_err(|_| Error("caller_disconnected"))?;
    let process_start = executor::process_start(pid)?;
    let manager = Proxy::new(
        connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|_| Error("session_service_unavailable"))?;
    let session_path: OwnedObjectPath = manager
        .call("GetSessionByPID", &(pid,))
        .await
        .map_err(|_| Error("inactive_or_remote_session"))?;
    let session = Proxy::new(
        connection,
        "org.freedesktop.login1",
        session_path.clone(),
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(|_| Error("session_service_unavailable"))?;
    let active: bool = session
        .get_property("Active")
        .await
        .map_err(|_| Error("inactive_or_remote_session"))?;
    let remote: bool = session
        .get_property("Remote")
        .await
        .map_err(|_| Error("inactive_or_remote_session"))?;
    let class: String = session
        .get_property("Class")
        .await
        .map_err(|_| Error("inactive_or_remote_session"))?;
    let user: (u32, OwnedObjectPath) = session
        .get_property("User")
        .await
        .map_err(|_| Error("inactive_or_remote_session"))?;
    if !active || remote || class != "user" || user.0 != uid {
        return Err(Error("inactive_or_remote_session"));
    }
    // Confirm the connection still exists after consulting logind. Unique bus
    // names are never recycled within a bus lifetime; plans are memory-only.
    let still_pid: u32 = bus
        .call("GetConnectionUnixProcessID", &(unique_name,))
        .await
        .map_err(|_| Error("caller_disconnected"))?;
    if still_pid != pid || executor::process_start(pid)? != process_start {
        return Err(Error("caller_changed"));
    }
    Ok(Caller {
        unique_name: unique_name.into(),
        uid,
        pid,
        process_start,
        session: session_path.to_string(),
    })
}

pub async fn authorize(connection: &Connection, caller: &Caller, action: &str) -> Result<()> {
    if ![
        "org.sentia.system.service-start",
        "org.sentia.system.service-stop",
        "org.sentia.system.service-restart",
        "org.sentia.system.service-enable",
        "org.sentia.system.process-terminate",
        "org.sentia.system.package-install",
        "org.sentia.system.package-remove",
        "org.sentia.system.package-update",
        "org.sentia.system.package-upgrade",
    ]
    .contains(&action)
    {
        return Err(Error("invalid_action"));
    }
    let authority = Proxy::new(
        connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await
    .map_err(|_| Error("authorization_unavailable"))?;
    let mut identity = HashMap::new();
    identity.insert("name", Value::from(caller.unique_name.as_str()));
    let details: HashMap<&str, &str> = HashMap::new();
    let body = (
        ("system-bus-name", identity),
        action,
        details,
        1_u32, // AllowUserInteraction, not an assertion of approval.
        "",
    );
    let check = authority.call::<_, _, (bool, bool, HashMap<String, String>)>(
        "CheckAuthorization", &body,
    );
    let (authorized, _, _) = tokio::time::timeout(Duration::from_secs(90), check)
        .await
        .map_err(|_| Error("authorization_timeout"))?
        .map_err(|_| Error("authorization_unavailable"))?;
    if !authorized {
        return Err(Error("authorization_denied"));
    }
    Ok(())
}

#[zbus::interface(name = "org.sentia.System1")]
impl SystemService {
    async fn prepare(
        &self,
        request: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        let operation = parse_request(request).map_err(dbus_error)?;
        let unique = header
            .sender()
            .ok_or_else(|| dbus_error(Error("missing_bus_sender")))?;
        let caller = identify(connection, unique.as_str()).await.map_err(dbus_error)?;
        let _guard = self
            .operation_lock
            .try_lock()
            .map_err(|_| dbus_error(Error("broker_busy")))?;
        let state = executor::snapshot(&operation).await.map_err(dbus_error)?;
        let current = identify(connection, unique.as_str()).await.map_err(dbus_error)?;
        if current != caller {
            return Err(dbus_error(Error("caller_changed")));
        }
        let prepared = self
            .store
            .lock()
            .await
            .prepare(caller, operation, state, now().map_err(dbus_error)?)
            .map_err(dbus_error)?;
        serde_json::to_string(&prepared)
            .map_err(|_| dbus_error(Error("serialization_failed")))
    }

    async fn apply(
        &self,
        plan_id: &str,
        digest: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        let unique = header
            .sender()
            .ok_or_else(|| dbus_error(Error("missing_bus_sender")))?;
        let caller = identify(connection, unique.as_str()).await.map_err(dbus_error)?;
        let _guard = self
            .operation_lock
            .try_lock()
            .map_err(|_| dbus_error(Error("broker_busy")))?;
        let plan = self
            .store
            .lock()
            .await
            .consume(plan_id, digest, &caller, now().map_err(dbus_error)?)
            .map_err(dbus_error)?;
        let before = executor::snapshot(&plan.operation).await.map_err(dbus_error)?;
        revalidate(&plan, &caller, &before, now().map_err(dbus_error)?)
            .map_err(dbus_error)?;
        if let Err(error) = authorize(connection, &caller, &plan.action_id).await {
            eprintln!("sentia_privileged event=authorization uid={} action={} result={}",
                caller.uid, plan.action_id, error.0);
            return Err(dbus_error(error));
        }
        let current = identify(connection, unique.as_str()).await.map_err(dbus_error)?;
        let after = executor::snapshot(&plan.operation).await.map_err(dbus_error)?;
        revalidate(&plan, &current, &after, now().map_err(dbus_error)?)
            .map_err(dbus_error)?;
        let result = executor::execute(&plan).await;
        eprintln!("sentia_privileged event=execution uid={} action={} result={}",
            caller.uid, plan.action_id,
            result.as_ref().map(|_| "completed").unwrap_or_else(|error| error.0));
        let result = result.map_err(dbus_error)?;
        serde_json::to_string(&result).map_err(|_| dbus_error(Error("serialization_failed")))
    }
}
