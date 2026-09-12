// SPDX-License-Identifier: Apache-2.0

use super::{
    finish_reason, insert_bearer, json_headers, parse_json, role_name, token_usage, ParsedResponse,
    ProviderAdapter, USER_AGENT,
};
use crate::{
    credential::SecretBytes,
    error::ProviderError,
    protocol::{InferenceRequest, ProviderId},
};
use reqwest::{
    header::{HeaderValue, USER_AGENT as USER_AGENT_HEADER},
    Client, Request,
};
use serde_json::{json, Map, Value};

pub(crate) struct DeepSeekAdapter;

impl ProviderAdapter for DeepSeekAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::DeepSeek
    }

    fn build_request(
        &self,
        client: &Client,
        request: &InferenceRequest,
        credential: &SecretBytes,
    ) -> Result<Request, ProviderError> {
        let mut headers = json_headers();
        insert_bearer(&mut headers, credential)?;
        headers.insert(USER_AGENT_HEADER, HeaderValue::from_static(USER_AGENT));

        let mut messages = Vec::with_capacity(request.messages.len() + usize::from(request.system.is_some()));
        if let Some(system) = &request.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.extend(request.messages.iter().map(|message| {
            json!({
                "role": role_name(message.role),
                "content": message.content,
            })
        }));
        let mut body = Map::from_iter([
            ("model".to_owned(), json!(request.model)),
            ("messages".to_owned(), Value::Array(messages)),
            ("max_tokens".to_owned(), json!(request.max_output_tokens)),
            ("stream".to_owned(), json!(false)),
        ]);
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }

        client
            .post("https://api.deepseek.com/chat/completions")
            .headers(headers)
            .json(&Value::Object(body))
            .build()
            .map_err(|_| ProviderError::process("request_build_failed", "The request could not be built."))
    }

    fn parse_success(&self, body: &[u8]) -> Result<ParsedResponse, ProviderError> {
        let value = parse_json(body)?;
        if value.get("object").and_then(Value::as_str) != Some("chat.completion") {
            return Err(ProviderError::malformed("unexpected_object"));
        }
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::malformed("missing_choices"))?;
        if choices.len() != 1 {
            return Err(ProviderError::malformed("unexpected_choice_count"));
        }
        let choice = &choices[0];
        let message = choice
            .get("message")
            .ok_or_else(|| ProviderError::malformed("missing_message"))?;
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            return Err(ProviderError::malformed("unexpected_message_role"));
        }
        if message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty())
        {
            return Err(ProviderError::unsupported(
                "provider_tool_output",
                "The provider returned a tool call even though tools were not requested.",
            ));
        }
        let text = message
            .get("content")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProviderError::malformed("empty_text_output"))?
            .to_owned();
        let usage = value.get("usage");

        Ok(ParsedResponse {
            text,
            finish_reason: finish_reason(
                choice.get("finish_reason").and_then(Value::as_str),
                false,
            ),
            incomplete: false,
            usage: token_usage(
                usage
                    .and_then(|item| item.get("prompt_tokens"))
                    .and_then(Value::as_u64),
                usage
                    .and_then(|item| item.get("completion_tokens"))
                    .and_then(Value::as_u64),
                usage
                    .and_then(|item| item.get("total_tokens"))
                    .and_then(Value::as_u64),
            ),
        })
    }
}
