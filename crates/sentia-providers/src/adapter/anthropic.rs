// SPDX-License-Identifier: Apache-2.0

use super::{
    api_key_header, finish_reason, json_headers, parse_json, role_name, token_usage, ParsedResponse,
    ProviderAdapter, USER_AGENT,
};
use crate::{
    credential::SecretBytes,
    error::ProviderError,
    protocol::{InferenceRequest, ProviderId},
};
use reqwest::{
    header::{HeaderName, HeaderValue, USER_AGENT as USER_AGENT_HEADER},
    Client, Request,
};
use serde_json::{json, Map, Value};

pub(crate) struct AnthropicAdapter;

impl ProviderAdapter for AnthropicAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    fn build_request(
        &self,
        client: &Client,
        request: &InferenceRequest,
        credential: &SecretBytes,
    ) -> Result<Request, ProviderError> {
        if request.temperature.is_some_and(|value| value > 1.0) {
            return Err(ProviderError::invalid(
                "invalid_temperature",
                "Anthropic Messages accepts temperature values between 0 and 1.",
            ));
        }

        let mut headers = json_headers();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            api_key_header(credential)?,
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(USER_AGENT_HEADER, HeaderValue::from_static(USER_AGENT));

        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                json!({
                    "role": role_name(message.role),
                    "content": message.content,
                })
            })
            .collect();
        let mut body = Map::from_iter([
            ("model".to_owned(), json!(request.model)),
            ("messages".to_owned(), Value::Array(messages)),
            ("max_tokens".to_owned(), json!(request.max_output_tokens)),
            ("stream".to_owned(), json!(false)),
        ]);
        if let Some(system) = &request.system {
            body.insert("system".to_owned(), json!(system));
        }
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }

        client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers)
            .json(&Value::Object(body))
            .build()
            .map_err(|_| ProviderError::process("request_build_failed", "The request could not be built."))
    }

    fn parse_success(&self, body: &[u8]) -> Result<ParsedResponse, ProviderError> {
        let value = parse_json(body)?;
        if value.get("type").and_then(Value::as_str) != Some("message")
            || value.get("role").and_then(Value::as_str) != Some("assistant")
        {
            return Err(ProviderError::malformed("unexpected_message_envelope"));
        }
        let content = value
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::malformed("missing_content"))?;
        let mut text = String::new();
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    text.push_str(
                        block
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| ProviderError::malformed("missing_text"))?,
                    );
                }
                Some("thinking" | "redacted_thinking") => {}
                _ => {
                    return Err(ProviderError::unsupported(
                        "provider_tool_or_unknown_output",
                        "The provider returned a tool call or unsupported content block.",
                    ));
                }
            }
        }
        if text.is_empty() {
            return Err(ProviderError::malformed("empty_text_output"));
        }

        let usage = value.get("usage");
        Ok(ParsedResponse {
            text,
            finish_reason: finish_reason(
                value.get("stop_reason").and_then(Value::as_str),
                false,
            ),
            incomplete: false,
            usage: token_usage(
                usage
                    .and_then(|item| item.get("input_tokens"))
                    .and_then(Value::as_u64),
                usage
                    .and_then(|item| item.get("output_tokens"))
                    .and_then(Value::as_u64),
                None,
            ),
        })
    }
}
