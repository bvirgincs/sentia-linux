// SPDX-License-Identifier: Apache-2.0

use sentia_providers::{worker, ProviderCapabilities, ProviderError, ProviderId};
use std::{env, process, str::FromStr};

enum Mode {
    Run(ProviderId),
    Capabilities(ProviderId),
}

#[tokio::main]
async fn main() {
    let result = match parse_args() {
        Ok(Mode::Run(provider)) => worker::run_stdio(provider).await,
        Ok(Mode::Capabilities(provider)) => {
            let capabilities = ProviderCapabilities::for_provider(provider);
            serde_json::to_writer(std::io::stdout(), &capabilities)
                .map_err(|_| {
                    ProviderError::process(
                        "response_write_failed",
                        "Capabilities could not be written.",
                    )
                })
                .map(|_| println!())
        }
        Err(error) => Err(error),
    };

    if let Err(error) = result {
        eprintln!("sentia-provider-worker: {error}");
        process::exit(1);
    }
}

fn parse_args() -> Result<Mode, ProviderError> {
    let mut provider = None;
    let mut capabilities = false;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--provider" => {
                let value = arguments.next().ok_or_else(|| {
                    ProviderError::invalid(
                        "missing_provider",
                        "--provider requires a provider name.",
                    )
                })?;
                provider = Some(ProviderId::from_str(&value)?);
            }
            "--capabilities" => capabilities = true,
            _ => {
                return Err(ProviderError::invalid(
                    "invalid_argument",
                    "Only --provider and --capabilities are accepted.",
                ));
            }
        }
    }
    let provider = provider.ok_or_else(|| {
        ProviderError::invalid("missing_provider", "--provider is required.")
    })?;
    Ok(if capabilities {
        Mode::Capabilities(provider)
    } else {
        Mode::Run(provider)
    })
}
