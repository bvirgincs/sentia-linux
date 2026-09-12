// SPDX-License-Identifier: Apache-2.0

mod anthropic;
mod deepseek;
mod gemini;
mod openai;
mod responses;
mod xai;

use crate::{
    credential::SecretBytes,
    error::{ProviderError, ProviderErrorKind},
    protocol::{FinishReason, InferenceRequest, ProviderId, TokenUsage},
};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE},
    Client, Request, StatusCode,
};
use serde_json::Value;
use std::sync::Arc;

pub(crate) use anthropic::AnthropicAdapter;
pub(crate) use deepseek::DeepSeekAdapter;
pub(crate) use gemini::GeminiAdapter;
pub(crate) use openai::OpenAiAdapter;
pub(crate) use xai::XaiAdapter;

pub(crate) const USER_AGENT: &str = "sentia-provider-worker/0.1";

#[derive(Debug)]
pub(crate) struct ParsedResponse {
    pub text: String,
    pub finish_reason: FinishReason,
    pub incomplete: bool,
    pub usage: Option<TokenUsage>,
}

pub(crate) trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> ProviderId;

    fn build_request(
        &self,
        client: &Client,
        request: &InferenceRequest,
        credential: &SecretBytes,
    ) -> Result<Request, ProviderError>;

    fn parse_success(&self, body: &[u8]) -> Result<ParsedResponse, ProviderError>;

    fn parse_error(&self, status: StatusCode, body: &[u8]) -> ProviderError {
        classify_http_error(status, body)
    }
}

pub(crate) fn for_provider(provider: ProviderId) -> Arc<dyn ProviderAdapter> {
    match provider {
        ProviderId::OpenAi => Arc::new(OpenAiAdapter),
        ProviderId::Anthropic => Arc::new(AnthropicAdapter),
        ProviderId::Gemini => Arc::new(GeminiAdapter),
        ProviderId::Xai => Arc::new(XaiAdapter),
        ProviderId::DeepSeek => Arc::new(DeepSeekAdapter),
    }
}

pub(crate) fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

pub(crate) fn bearer_header(secret: &SecretBytes) -> Result<HeaderValue, ProviderError> {
    let mut value = Vec::with_capacity(7 + secret.expose().len());
    value.extend_from_slice(b"Bearer ");
    value.extend_from_slice(secret.expose());
    let result = HeaderValue::from_bytes(&value);
    value.fill(0);
    let mut header = result.map_err(|_| authentication_header_error())?;
    header.set_sensitive(true);
    Ok(header)
}

pub(crate) fn api_key_header(secret: &SecretBytes) -> Result<HeaderValue, ProviderError> {
    let mut header =
        HeaderValue::from_bytes(secret.expose()).map_err(|_| authentication_header_error())?;
    header.set_sensitive(true);
    Ok(header)
}

pub(crate) fn insert_bearer(
    headers: &mut HeaderMap,
    secret: &SecretBytes,
) -> Result<(), ProviderError> {
    headers.insert(AUTHORIZATION, bearer_header(secret)?);
    Ok(())
}

fn authentication_header_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Authentication,
        "invalid_credential",
        "The provider credential cannot be represented as an HTTP header.",
        false,
    )
}

pub(crate) fn parse_json(body: &[u8]) -> Result<Value, ProviderError> {
    serde_json::from_slice(body).map_err(|_| ProviderError::malformed("invalid_json"))
}

pub(crate) fn token_usage(
    input: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
) -> Option<TokenUsage> {
    if input.is_none() && output.is_none() && total.is_none() {
        None
    } else {
        Some(TokenUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: total,
        })
    }
}

pub(crate) fn role_name(role: crate::protocol::Role) -> &'static str {
    match role {
        crate::protocol::Role::User => "user",
        crate::protocol::Role::Assistant => "assistant",
    }
}

pub(crate) fn finish_reason(value: Option<&str>, incomplete: bool) -> FinishReason {
    if incomplete {
        return FinishReason::Incomplete;
    }
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "stop" | "end_turn" | "stop_sequence" | "complete" | "completed" => FinishReason::Stop,
        "length" | "max_tokens" | "max_output_tokens" => FinishReason::Length,
        "refusal" | "safety" | "content_filter" | "blocked" => FinishReason::Refusal,
        "" => FinishReason::Other,
        _ => FinishReason::Other,
    }
}

pub(crate) fn classify_http_error(status: StatusCode, body: &[u8]) -> ProviderError {
    let code = safe_vendor_code(body).unwrap_or_else(|| format!("http_{}", status.as_u16()));
    let normalized_code = code.to_ascii_lowercase();
    let expired = normalized_code.contains("expir")
        || serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_ascii_lowercase)
            })
            .is_some_and(|message| message.contains("expir"));
    let unsupported_model = normalized_code.contains("model")
        && (normalized_code.contains("unsupported")
            || normalized_code.contains("not_found")
            || normalized_code.contains("deprecated"));

    match status.as_u16() {
        401 if expired => ProviderError::new(
            ProviderErrorKind::AuthenticationExpired,
            "authentication_expired",
            "The provider credential has expired.",
            false,
        ),
        401 | 403 => ProviderError::new(
            ProviderErrorKind::Authentication,
            code,
            "The provider rejected the credential or its permissions.",
            false,
        ),
        402 | 429 => ProviderError::new(
            ProviderErrorKind::Quota,
            code,
            "The provider reported a billing, quota, or rate limit.",
            status.as_u16() == 429,
        ),
        408 | 504 => ProviderError::new(
            ProviderErrorKind::Timeout,
            code,
            "The provider timed out.",
            true,
        ),
        400 | 404 if unsupported_model => ProviderError::new(
            ProviderErrorKind::UnsupportedCapability,
            code,
            "The provider does not support the configured model for this request.",
            false,
        ),
        400 | 404 | 409 | 413 | 422 => ProviderError::new(
            ProviderErrorKind::InvalidRequest,
            code,
            "The provider rejected the configured model or request.",
            false,
        ),
        500..=599 => ProviderError::new(
            ProviderErrorKind::Network,
            code,
            "The provider service is temporarily unavailable.",
            true,
        ),
        _ => ProviderError::new(
            ProviderErrorKind::Network,
            code,
            "The provider returned an unexpected HTTP status.",
            false,
        ),
    }
}

fn safe_vendor_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let candidates = [
        value.pointer("/error/code"),
        value.pointer("/error/type"),
        value.get("type"),
        value.pointer("/error/status"),
    ];
    let code = candidates
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .and_then(sanitize_code);
    code
}

fn sanitize_code(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{InferenceRequest, Message, Role};

    const FIXTURE_SECRET: &[u8] = b"fixture-secret-never-emit";

    fn request(provider: ProviderId) -> InferenceRequest {
        InferenceRequest {
            request_id: "fixture-request".to_owned(),
            provider,
            model: "configured-model".to_owned(),
            system: Some("system guidance".to_owned()),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "hello".to_owned(),
                },
                Message {
                    role: Role::Assistant,
                    content: "hi".to_owned(),
                },
                Message {
                    role: Role::User,
                    content: "answer briefly".to_owned(),
                },
            ],
            max_output_tokens: 32,
            temperature: Some(0.25),
            timeout_ms: Some(2_000),
            stream: false,
        }
    }

    fn assert_request_fixture(
        adapter: &dyn ProviderAdapter,
        provider: ProviderId,
        fixture: &str,
    ) {
        let client = reqwest::Client::builder().build().unwrap();
        let secret = SecretBytes::for_test(FIXTURE_SECRET);
        let request = adapter
            .build_request(&client, &request(provider), &secret)
            .unwrap();
        let actual: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let expected: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(actual, expected);
        assert!(request
            .headers()
            .values()
            .any(reqwest::header::HeaderValue::is_sensitive));
        assert!(!format!("{:?}", request.headers()).contains("fixture-secret-never-emit"));
    }

    fn assert_response_fixture(
        adapter: &dyn ProviderAdapter,
        fixture: &str,
        expected_text: &str,
    ) {
        let response = adapter.parse_success(fixture.as_bytes()).unwrap();
        assert_eq!(response.text, expected_text);
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert!(!response.incomplete);
        assert!(response.usage.is_some());
    }

    #[test]
    fn request_bodies_match_reviewed_fixtures() {
        assert_request_fixture(
            &OpenAiAdapter,
            ProviderId::OpenAi,
            include_str!("../../../../tests/providers/fixtures/openai/request.json"),
        );
        assert_request_fixture(
            &AnthropicAdapter,
            ProviderId::Anthropic,
            include_str!("../../../../tests/providers/fixtures/anthropic/request.json"),
        );
        assert_request_fixture(
            &GeminiAdapter,
            ProviderId::Gemini,
            include_str!("../../../../tests/providers/fixtures/gemini/request.json"),
        );
        assert_request_fixture(
            &XaiAdapter,
            ProviderId::Xai,
            include_str!("../../../../tests/providers/fixtures/xai/request.json"),
        );
        assert_request_fixture(
            &DeepSeekAdapter,
            ProviderId::DeepSeek,
            include_str!("../../../../tests/providers/fixtures/deepseek/request.json"),
        );
    }

    #[test]
    fn responses_match_reviewed_fixtures() {
        assert_response_fixture(
            &OpenAiAdapter,
            include_str!("../../../../tests/providers/fixtures/openai/response.json"),
            "fixture response",
        );
        assert_response_fixture(
            &AnthropicAdapter,
            include_str!("../../../../tests/providers/fixtures/anthropic/response.json"),
            "fixture response",
        );
        assert_response_fixture(
            &GeminiAdapter,
            include_str!("../../../../tests/providers/fixtures/gemini/response.json"),
            "fixture response",
        );
        assert_response_fixture(
            &XaiAdapter,
            include_str!("../../../../tests/providers/fixtures/xai/response.json"),
            "fixture response",
        );
        assert_response_fixture(
            &DeepSeekAdapter,
            include_str!("../../../../tests/providers/fixtures/deepseek/response.json"),
            "fixture response",
        );
    }

    #[test]
    fn errors_are_typed_without_vendor_messages() {
        let cases: [(&dyn ProviderAdapter, StatusCode, &str, ProviderErrorKind); 5] = [
            (
                &OpenAiAdapter,
                StatusCode::UNAUTHORIZED,
                include_str!("../../../../tests/providers/fixtures/openai/error.json"),
                ProviderErrorKind::Authentication,
            ),
            (
                &AnthropicAdapter,
                StatusCode::TOO_MANY_REQUESTS,
                include_str!("../../../../tests/providers/fixtures/anthropic/error.json"),
                ProviderErrorKind::Quota,
            ),
            (
                &GeminiAdapter,
                StatusCode::FORBIDDEN,
                include_str!("../../../../tests/providers/fixtures/gemini/error.json"),
                ProviderErrorKind::Authentication,
            ),
            (
                &XaiAdapter,
                StatusCode::TOO_MANY_REQUESTS,
                include_str!("../../../../tests/providers/fixtures/xai/error.json"),
                ProviderErrorKind::Quota,
            ),
            (
                &DeepSeekAdapter,
                StatusCode::PAYMENT_REQUIRED,
                include_str!("../../../../tests/providers/fixtures/deepseek/error.json"),
                ProviderErrorKind::Quota,
            ),
        ];
        for (adapter, status, fixture, expected_kind) in cases {
            let error = adapter.parse_error(status, fixture.as_bytes());
            assert_eq!(error.kind, expected_kind);
            let encoded = serde_json::to_string(&error).unwrap();
            assert!(!encoded.contains("fixture-secret-never-emit"));
            assert!(!encoded.contains("vendor detail"));
        }
    }

    #[test]
    fn rejects_tool_outputs_from_every_contract() {
        let cases: [(&dyn ProviderAdapter, &str); 5] = [
            (
                &OpenAiAdapter,
                r#"{"object":"response","status":"completed","output":[{"type":"function_call","name":"shell","arguments":"{}"}]}"#,
            ),
            (
                &AnthropicAdapter,
                r#"{"type":"message","role":"assistant","content":[{"type":"tool_use","name":"shell","input":{}}]}"#,
            ),
            (
                &GeminiAdapter,
                r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"shell","args":{}}}]},"finishReason":"STOP"}]}"#,
            ),
            (
                &XaiAdapter,
                r#"{"object":"response","status":"completed","output":[{"type":"web_search_call","status":"completed"}]}"#,
            ),
            (
                &DeepSeekAdapter,
                r#"{"object":"chat.completion","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"type":"function","function":{"name":"shell","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
            ),
        ];
        for (adapter, body) in cases {
            assert_eq!(
                adapter.parse_success(body.as_bytes()).unwrap_err().kind,
                ProviderErrorKind::UnsupportedCapability
            );
        }
    }

    #[test]
    fn does_not_return_vendor_message_or_secret() {
        let secret = "fixture-secret-never-emit";
        let body = format!(
            r#"{{"error":{{"type":"authentication_error","message":"bad {secret}"}}}}"#
        );
        let error = classify_http_error(StatusCode::UNAUTHORIZED, body.as_bytes());
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(secret));
        assert!(!serialized.contains("bad"));
        assert_eq!(error.kind, ProviderErrorKind::Authentication);
    }

    #[test]
    fn authorization_headers_are_sensitive() {
        let secret = SecretBytes::for_test(b"fixture-secret-never-emit");
        let header = bearer_header(&secret).unwrap();
        assert!(header.is_sensitive());
    }

    #[test]
    fn maps_expired_auth_without_exposing_vendor_text() {
        let error = classify_http_error(
            StatusCode::UNAUTHORIZED,
            br#"{"error":{"type":"authentication_error","message":"API key expired: fixture-secret-never-emit"}}"#,
        );
        assert_eq!(error.kind, ProviderErrorKind::AuthenticationExpired);
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("fixture-secret-never-emit"));
    }
}
