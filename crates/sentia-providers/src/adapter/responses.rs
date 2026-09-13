// SPDX-License-Identifier: Apache-2.0

use super::{finish_reason, parse_json, token_usage, ParsedResponse};
use crate::error::ProviderError;

pub(super) fn parse(body: &[u8]) -> Result<ParsedResponse, ProviderError> {
    let value = parse_json(body)?;
    if value.get("object").and_then(|item| item.as_str()) != Some("response") {
        return Err(ProviderError::malformed("unexpected_object"));
    }

    let status = value
        .get("status")
        .and_then(|item| item.as_str())
        .ok_or_else(|| ProviderError::malformed("missing_status"))?;
    if !matches!(status, "completed" | "incomplete") {
        return Err(ProviderError::malformed("unexpected_response_status"));
    }
    let incomplete = status == "incomplete";

    let output = value
        .get("output")
        .and_then(|item| item.as_array())
        .ok_or_else(|| ProviderError::malformed("missing_output"))?;
    let mut text = String::new();
    let mut refusal = false;

    for item in output {
        match item.get("type").and_then(|kind| kind.as_str()) {
            Some("reasoning") => {}
            Some("message") => {
                if item.get("role").and_then(|role| role.as_str()) != Some("assistant") {
                    return Err(ProviderError::malformed("unexpected_message_role"));
                }
                let content = item
                    .get("content")
                    .and_then(|content| content.as_array())
                    .ok_or_else(|| ProviderError::malformed("missing_message_content"))?;
                for block in content {
                    match block.get("type").and_then(|kind| kind.as_str()) {
                        Some("output_text") => {
                            let block_text = block
                                .get("text")
                                .and_then(|value| value.as_str())
                                .ok_or_else(|| ProviderError::malformed("missing_output_text"))?;
                            text.push_str(block_text);
                        }
                        Some("refusal") => {
                            let block_text = block
                                .get("refusal")
                                .or_else(|| block.get("text"))
                                .and_then(|value| value.as_str())
                                .ok_or_else(|| ProviderError::malformed("missing_refusal_text"))?;
                            text.push_str(block_text);
                            refusal = true;
                        }
                        _ => {
                            return Err(ProviderError::unsupported(
                                "provider_content_not_text",
                                "The provider returned non-text content.",
                            ));
                        }
                    }
                }
            }
            _ => {
                return Err(ProviderError::unsupported(
                    "provider_tool_or_unknown_output",
                    "The provider returned a tool call or unsupported output item.",
                ));
            }
        }
    }

    if text.is_empty() {
        return Err(ProviderError::malformed("empty_text_output"));
    }

    let usage = value.get("usage");
    let usage = token_usage(
        usage
            .and_then(|value| value.get("input_tokens"))
            .and_then(|value| value.as_u64()),
        usage
            .and_then(|value| value.get("output_tokens"))
            .and_then(|value| value.as_u64()),
        usage
            .and_then(|value| value.get("total_tokens"))
            .and_then(|value| value.as_u64()),
    );
    // A completed Responses API result carries no incomplete_details, so the
    // top-level status is the terminal signal. Without this the normal success
    // case mapped to FinishReason::Other.
    let reason = value
        .pointer("/incomplete_details/reason")
        .and_then(|value| value.as_str())
        .unwrap_or(status);

    Ok(ParsedResponse {
        text,
        finish_reason: if refusal {
            crate::protocol::FinishReason::Refusal
        } else {
            finish_reason(Some(reason), incomplete)
        },
        incomplete,
        usage,
    })
}
