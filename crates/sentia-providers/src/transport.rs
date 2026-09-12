// SPDX-License-Identifier: Apache-2.0

use crate::{
    adapter::ProviderAdapter,
    credential::SecretBytes,
    error::{ProviderError, ProviderErrorKind},
    protocol::{InferenceRequest, InferenceResponse, ProviderId, MAX_RESPONSE_BYTES},
};
use futures_util::StreamExt;
use reqwest::{
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    redirect::Policy,
    Client, Response,
};
use std::sync::Arc;
use tokio::time::timeout;
use url::Url;

#[derive(Clone)]
pub(crate) struct ProviderTransport {
    client: Client,
}

impl ProviderTransport {
    pub(crate) fn new() -> Result<Self, ProviderError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|_| {
                ProviderError::process(
                    "http_client_unavailable",
                    "The HTTPS client could not be initialized.",
                )
            })?;
        Ok(Self { client })
    }

    pub(crate) async fn infer(
        &self,
        adapter: Arc<dyn ProviderAdapter>,
        request: InferenceRequest,
        credential: Arc<SecretBytes>,
    ) -> Result<InferenceResponse, ProviderError> {
        let provider = adapter.provider();
        let model = request.model.clone();
        let request_timeout = request.timeout();
        let http_request = adapter.build_request(&self.client, &request, &credential)?;
        validate_destination(provider, http_request.url())?;

        let operation = async {
            let response = self
                .client
                .execute(http_request)
                .await
                .map_err(classify_transport_error)?;
            let status = response.status();
            if status.is_redirection() {
                return Err(ProviderError::new(
                    ProviderErrorKind::Network,
                    "unexpected_redirect",
                    "The provider attempted an unexpected redirect.",
                    false,
                ));
            }
            let is_json = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
                });
            let mut body = read_bounded_body(response).await?;
            if status.is_success() {
                if !is_json {
                    body.fill(0);
                    return Err(ProviderError::malformed("unexpected_content_type"));
                }
                let parsed = adapter.parse_success(&body);
                body.fill(0);
                parsed
            } else {
                let error = adapter.parse_error(status, &body);
                body.fill(0);
                Err(error)
            }
        };

        let parsed = timeout(request_timeout, operation).await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Timeout,
                "request_timeout",
                "The provider request exceeded its configured deadline.",
                true,
            )
        })??;

        Ok(InferenceResponse {
            provider,
            requested_model: model,
            text: parsed.text,
            finish_reason: parsed.finish_reason,
            incomplete: parsed.incomplete,
            usage: parsed.usage,
        })
    }
}

async fn read_bounded_body(response: Response) -> Result<Vec<u8>, ProviderError> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(ProviderError::malformed("response_too_large"));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_transport_error)?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            body.fill(0);
            return Err(ProviderError::malformed("response_too_large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_destination(provider: ProviderId, url: &Url) -> Result<(), ProviderError> {
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(destination_error());
    }

    let allowed = match provider {
        ProviderId::OpenAi => {
            url.host_str() == Some("api.openai.com") && url.path() == "/v1/responses"
        }
        ProviderId::Anthropic => {
            url.host_str() == Some("api.anthropic.com") && url.path() == "/v1/messages"
        }
        ProviderId::Gemini => {
            url.host_str() == Some("generativelanguage.googleapis.com")
                && url.path().starts_with("/v1beta/models/")
                && url.path().ends_with(":generateContent")
        }
        ProviderId::Xai => {
            url.host_str() == Some("api.x.ai") && url.path() == "/v1/responses"
        }
        ProviderId::DeepSeek => {
            url.host_str() == Some("api.deepseek.com") && url.path() == "/chat/completions"
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(destination_error())
    }
}

fn destination_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Network,
        "destination_not_allowlisted",
        "The provider destination is not allowlisted.",
        false,
    )
}

fn classify_transport_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        return ProviderError::new(
            ProviderErrorKind::Timeout,
            "transport_timeout",
            "The HTTPS transport timed out.",
            true,
        );
    }

    let internal = error.to_string().to_ascii_lowercase();
    if internal.contains("dns")
        || internal.contains("resolve")
        || internal.contains("name or service not known")
    {
        ProviderError::new(
            ProviderErrorKind::Dns,
            "dns_failure",
            "The provider hostname could not be resolved.",
            true,
        )
    } else {
        ProviderError::new(
            ProviderErrorKind::Network,
            "network_failure",
            "The HTTPS connection to the provider failed.",
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_rejects_endpoint_changes() {
        let valid = Url::parse("https://api.openai.com/v1/responses").unwrap();
        assert!(validate_destination(ProviderId::OpenAi, &valid).is_ok());

        for endpoint in [
            "http://api.openai.com/v1/responses",
            "https://api.openai.com:8443/v1/responses",
            "https://api.openai.com/v1/responses?target=private",
            "https://127.0.0.1/v1/responses",
            "https://api.openai.com.evil.example/v1/responses",
        ] {
            let url = Url::parse(endpoint).unwrap();
            assert!(validate_destination(ProviderId::OpenAi, &url).is_err());
        }
    }
}
