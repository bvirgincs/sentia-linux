// SPDX-License-Identifier: Apache-2.0
use sentia_privileged::{service::SystemService, BUS_NAME, OBJECT_PATH};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("sentia-privileged must run as the system service".into());
    }
    // Never accept an environment-supplied bus address while running as root.
    let _connection = zbus::connection::Builder::address("unix:path=/run/dbus/system_bus_socket")?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, SystemService::default())?
        .build()
        .await?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}
