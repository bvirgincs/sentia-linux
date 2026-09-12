// SPDX-License-Identifier: Apache-2.0

use super::{
    api_key_header, finish_reason, json_headers, parse_json, token_usage, ParsedResponse,
    ProviderAdapter, USER_AGENT,
};
use crate::{
    credential::SecretBytes,
    error::ProviderError,
    protocol::{InferenceRequest, ProviderId, Role},
};
use reqwest::{
    header::{HeaderName, HeaderValue, USER_AGENT as USER_AGENT_HEADER},
    Client, Request,
};
use serde_json::{json, Map, Value};

pub(crate) struct GeminiAdapter;

impl ProviderAdapter for GeminiAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn build_request(
        &self,
        client: &Client,
        request: &InferenceRequest,
        credential: &SecretBytes,
    ) -> Result<Request, ProviderError> {
        let mut headers = json_headers();
        headers.insert(
            HeaderName::from_static("x-goog-api-key"),
            api_key_header(credential)?,
        );
        headers.insert(USER_AGENT_HEADER, HeaderValue::from_static(USER_AGENT));

        let contents: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                json!({
                    "role": match message.role {
                        Role::User => "user",
                        Role::Assistant => "model",
                    },
                    "parts": [{"text": message.content}],
                })
            })
            .collect();
        let mut generation_config = Map::from_iter([(
            "maxOutputTokens".to_owned(),
            json!(request.max_output_tokens),
        )]);
        if let Some(temperature) = request.temperature {
            generation_config.insert("temperature".to_owned(), json!(temperature));
        }
        let mut body = Map::from_iter([
            ("contents".to_owned(), Value::Array(contents)),
            (
                "generationConfig".to_owned(),
                Value::Object(generation_config),
            ),
        ]);
        if let Some(system) = &request.system {
            body.insert(
                "system_instruction".to_owned(),
                json!({"parts": [{"text": system}]}),
            );
        }

        let endpoint = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            request.model
        );
        client
            .post(endpoint)
            .headers(headers)
            .json(&Value::Object(body))
            .build()
            .map_err(|_| ProviderError::process("request_build_failed", "The request could not be built."))
    }

    fn parse_success(&self, body: &[u8]) -> Result<ParsedResponse, ProviderError> {
        let value = parse_json(body)?;
        let candidates = value
            .get("candidates")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                if value.pointer("/promptFeedback/blockReason").is_some() {
                    ProviderError::unsupported(
                        "provider_blocked_prompt",
                        "The provider declined to generate text for this request.",
                    )
                } else {
                    ProviderError::malformed("missing_candidates")
                }
            })?;
        if candidates.len() != 1 {
            return Err(ProviderError::malformed("unexpected_candidate_count"));
        }
        let candidate = &candidates[0];
        let parts = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::malformed("missing_content_parts"))?;
        let mut text = String::new();
        for part in parts {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                text.push_str(part_text);
                continue;
            }
            return Err(ProviderError::unsupported(
                "provider_tool_or_non_text_output",
                "The provider returned a tool call or non-text output.",
            ));
        }
        if text.is_empty() {
            return Err(ProviderError::malformed("empty_text_output"));
        }

        let usage = value.get("usageMetadata");
        Ok(ParsedResponse {
            text,
            finish_reason: finish_reason(
                candidate.get("finishReason").and_then(Value::as_str),
                false,
            ),
            incomplete: false,
            usage: token_usage(
                usage
                    .and_then(|item| item.get("promptTokenCount"))
                    .and_then(Value::as_u64),
                usage
                    .and_then(|item| item.get("candidatesTokenCount"))
                    .and_then(Value::as_u64),
                usage
                    .and_then(|item| item.get("totalTokenCount"))
                    .and_then(Value::as_u64),
            ),
        })
    }
}
