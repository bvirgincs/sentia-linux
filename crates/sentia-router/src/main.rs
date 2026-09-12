use sentia_router::{
    config::RouterConfig, server, settings::SettingsStore, RouterBuilder,
};
use std::{io, sync::Arc};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> io::Result<()> {
    let config = RouterConfig::load(None)?;
    let settings = SettingsStore::load_default()?;
    let router = Arc::new(RouterBuilder::standalone(config, settings).build()?);
    let shutdown = CancellationToken::new();
    install_shutdown_handler(shutdown.clone());

    let self_test_router = router.clone();
    tokio::spawn(async move {
        let event = self_test_router.status("startup-self-test").await;
        if let sentia_router::api::InternalEvent::Control {
            response:
                sentia_router::api::ControlResponse::Status { display, .. },
            ..
        } = event
        {
            eprintln!("sentia-router: {display}");
        }
    });

    server::run(router, shutdown).await
}

fn install_shutdown_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut terminate =
                signal(SignalKind::terminate()).expect("install SIGTERM handler");
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        shutdown.cancel();
    });
}
