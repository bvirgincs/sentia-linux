mod config;
mod llama;
mod queue;
mod server;

use crate::{config::BrokerConfig, server::Broker};
use std::{io, sync::Arc};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> io::Result<()> {
    let config = BrokerConfig::load(None)?;
    let broker = Arc::new(Broker::new(config));
    let shutdown = CancellationToken::new();
    install_shutdown_handler(shutdown.clone());
    broker.run(shutdown).await
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
