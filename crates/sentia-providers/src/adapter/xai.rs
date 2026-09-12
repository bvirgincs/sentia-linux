// SPDX-License-Identifier: Apache-2.0

use super::{
    insert_bearer, json_headers, responses, role_name, ParsedResponse, ProviderAdapter, USER_AGENT,
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

pub(crate) struct XaiAdapter;

impl ProviderAdapter for XaiAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::Xai
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

        let mut input = Vec::with_capacity(request.messages.len() + usize::from(request.system.is_some()));
        if let Some(system) = &request.system {
            input.push(json!({"role": "system", "content": system}));
        }
        input.extend(request.messages.iter().map(|message| {
            json!({
                "role": role_name(message.role),
                "content": message.content,
            })
        }));

        let mut body = Map::from_iter([
            ("model".to_owned(), json!(request.model)),
            ("input".to_owned(), Value::Array(input)),
            (
                "max_output_tokens".to_owned(),
                json!(request.max_output_tokens),
            ),
            ("store".to_owned(), json!(false)),
            ("stream".to_owned(), json!(false)),
            ("tools".to_owned(), json!([])),
            ("tool_choice".to_owned(), json!("none")),
            (
                "search_parameters".to_owned(),
                json!({"mode": "off"}),
            ),
        ]);
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }

        client
            .post("https://api.x.ai/v1/responses")
            .headers(headers)
            .json(&Value::Object(body))
            .build()
            .map_err(|_| ProviderError::process("request_build_failed", "The request could not be built."))
    }

    fn parse_success(&self, body: &[u8]) -> Result<ParsedResponse, ProviderError> {
        responses::parse(body)
    }
}
