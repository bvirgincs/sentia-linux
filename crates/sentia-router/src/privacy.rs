use crate::api::{ContextItem, PrivacyCategory};
use sentia_protocol::router::ConsentToken;
use sentia_protocol::BoundedString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CONSENT_LIFETIME: Duration = Duration::from_secs(5 * 60);
const MAX_CONTEXT_ITEM_BYTES: usize = 64 * 1024;
const MAX_TOTAL_CONTEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedPayload {
    pub provider: String,
    pub question: String,
    pub context: Vec<ContextItem>,
    pub max_tokens: u32,
    pub redactions: Vec<String>,
}

impl SanitizedPayload {
    pub fn canonical_value(&self) -> Value {
        json!({
            "context": self.context,
            "max_tokens": self.max_tokens,
            "provider": self.provider,
            "question": self.question,
        })
    }

    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(&self.canonical_value()).expect("value serialization");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Clone)]
pub struct PrivacyGuard {
    consent: Arc<Mutex<ConsentState>>,
}

struct ConsentState {
    secret: [u8; 32],
    entries: HashMap<String, ConsentEntry>,
}

struct ConsentEntry {
    provider: String,
    digest: String,
    expires_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct ConsentPreview {
    pub token: ConsentToken,
}

#[derive(Debug, thiserror::Error)]
pub enum PrivacyError {
    #[error("context category is not permitted for remote disclosure: {0:?}")]
    CategoryDenied(PrivacyCategory),
    #[error("context source is forbidden from remote disclosure")]
    ForbiddenSource,
    #[error("remote consent is missing, expired, already used, or does not match the payload")]
    InvalidConsent,
    #[error("request exceeds privacy boundary limits")]
    TooLarge,
}

impl PrivacyGuard {
    pub fn new() -> std::io::Result<Self> {
        let mut secret = [0_u8; 32];
        File::open("/dev/urandom")?.read_exact(&mut secret)?;
        Ok(Self {
            consent: Arc::new(Mutex::new(ConsentState {
                secret,
                entries: HashMap::new(),
            })),
        })
    }

    pub fn sanitize(
        &self,
        provider: &str,
        question: &str,
        context: &[ContextItem],
        max_tokens: u32,
        allowed_categories: Option<&BTreeSet<PrivacyCategory>>,
    ) -> Result<SanitizedPayload, PrivacyError> {
        if question.len() > MAX_CONTEXT_ITEM_BYTES {
            return Err(PrivacyError::TooLarge);
        }
        if let Some(allowed) = allowed_categories {
            if !allowed.contains(&PrivacyCategory::UserPrompt) {
                return Err(PrivacyError::CategoryDenied(PrivacyCategory::UserPrompt));
            }
        }
        let (question, mut redactions) = redact_secrets(question);
        let mut sanitized = Vec::with_capacity(context.len());
        let mut total = question.len();
        for item in context {
            if forbidden_source(&item.source) {
                return Err(PrivacyError::ForbiddenSource);
            }
            if let Some(allowed) = allowed_categories {
                if !allowed.contains(&item.category) {
                    return Err(PrivacyError::CategoryDenied(item.category.clone()));
                }
            }
            if item.content.len() > MAX_CONTEXT_ITEM_BYTES {
                return Err(PrivacyError::TooLarge);
            }
            total = total.saturating_add(item.content.len());
            if total > MAX_TOTAL_CONTEXT_BYTES {
                return Err(PrivacyError::TooLarge);
            }
            let (content, found) = redact_secrets(&item.content);
            redactions.extend(found);
            sanitized.push(ContextItem {
                category: item.category.clone(),
                source: safe_source(&item.source),
                content,
            });
        }
        redactions.sort();
        redactions.dedup();
        Ok(SanitizedPayload {
            provider: provider.to_owned(),
            question,
            context: sanitized,
            max_tokens,
            redactions,
        })
    }

    pub fn preview(&self, payload: &SanitizedPayload) -> Result<ConsentPreview, PrivacyError> {
        let now = unix_ms();
        let expires_at_ms = now + CONSENT_LIFETIME.as_millis() as u64;
        let digest = payload.digest();
        let mut state = self.consent.lock().expect("consent lock poisoned");
        state.entries.retain(|_, value| value.expires_at_ms >= now);
        let nonce = state.entries.len() as u64 ^ now ^ std::process::id() as u64;
        let mut hasher = Sha256::new();
        hasher.update(state.secret);
        hasher.update(payload.provider.as_bytes());
        hasher.update(digest.as_bytes());
        hasher.update(nonce.to_le_bytes());
        let token = hex::encode(hasher.finalize());
        state.entries.insert(
            token.clone(),
            ConsentEntry {
                provider: payload.provider.clone(),
                digest,
                expires_at_ms,
            },
        );
        Ok(ConsentPreview {
            token: ConsentToken {
                token_id: BoundedString::new(token).map_err(|_| PrivacyError::TooLarge)?,
                payload_sha256: BoundedString::new(payload.digest())
                    .map_err(|_| PrivacyError::TooLarge)?,
                provider_id: Some(
                    BoundedString::new(payload.provider.clone())
                        .map_err(|_| PrivacyError::TooLarge)?,
                ),
                issued_at_ms: now,
                expires_at_ms,
            },
        })
    }

    pub fn consume_consent(
        &self,
        token: Option<&ConsentToken>,
        payload: &SanitizedPayload,
    ) -> Result<(), PrivacyError> {
        let token = token.ok_or(PrivacyError::InvalidConsent)?;
        let mut state = self.consent.lock().expect("consent lock poisoned");
        let entry = state
            .entries
            .remove(token.token_id.as_str())
            .ok_or(PrivacyError::InvalidConsent)?;
        if token.expires_at_ms < unix_ms()
            || token.expires_at_ms != entry.expires_at_ms
            || token.issued_at_ms >= token.expires_at_ms
            || entry.provider != payload.provider
            || entry.digest != payload.digest()
            || token.payload_sha256.as_str() != entry.digest
            || token
                .provider_id
                .as_ref()
                .map(|value| value.as_str())
                != Some(entry.provider.as_str())
        {
            return Err(PrivacyError::InvalidConsent);
        }
        Ok(())
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn safe_source(source: &str) -> String {
    if source.starts_with('/') {
        source
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("local-context")
            .to_owned()
    } else {
        source.chars().take(256).collect()
    }
}

fn forbidden_source(source: &str) -> bool {
    let normalized = source.to_ascii_lowercase();
    let forbidden = [
        "/.ssh/",
        "/.gnupg/",
        "/.aws/",
        "/.config/gcloud/",
        "/.config/gh/",
        "/.local/share/keyrings/",
        "/keyrings/",
        "/cookies",
        "/login data",
        "/shadow",
        "/etc/ssl/private/",
        "command_history",
        ".bash_history",
        ".zsh_history",
        "environment",
    ];
    forbidden.iter().any(|needle| normalized.contains(needle))
}

fn redact_secrets(input: &str) -> (String, Vec<String>) {
    let lowercase = input.to_ascii_lowercase();
    if lowercase.contains("-----begin") && lowercase.contains("private key-----") {
        return (
            "[REDACTED PRIVATE KEY]".to_owned(),
            vec!["private_key".to_owned()],
        );
    }
    let mut output = String::with_capacity(input.len());
    let mut labels = Vec::new();
    let mut redact_next = false;
    for token in input.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end_matches(char::is_whitespace);
        let suffix = &token[trimmed.len()..];
        if redact_next && trimmed.eq_ignore_ascii_case("bearer") {
            output.push_str(trimmed);
            output.push_str(suffix);
        } else if redact_next {
            output.push_str("[REDACTED]");
            output.push_str(suffix);
            labels.push("authorization_token".to_owned());
            redact_next = false;
        } else if trimmed.eq_ignore_ascii_case("authorization:")
            || trimmed.eq_ignore_ascii_case("bearer")
        {
            output.push_str(trimmed);
            output.push_str(suffix);
            redact_next = true;
        } else if let Some(label) = secret_label(trimmed) {
            output.push_str("[REDACTED]");
            output.push_str(suffix);
            labels.push(label.to_owned());
        } else {
            output.push_str(token);
        }
    }
    (output, labels)
}

fn secret_label(token: &str) -> Option<&'static str> {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("authorization:")
        || lower.starts_with("bearer:")
        || lower.starts_with("password=")
        || lower.starts_with("passwd=")
        || lower.starts_with("token=")
        || lower.starts_with("secret=")
        || lower.starts_with("api_key=")
        || lower.starts_with("apikey=")
        || lower.starts_with("aws_secret_access_key=")
        || lower.starts_with("aws_session_token=")
        || lower.starts_with("github_token=")
    {
        return Some("credential_assignment");
    }
    if token.starts_with("sk-") && token.len() >= 20 {
        return Some("api_key");
    }
    if token.starts_with("ghp_") || token.starts_with("github_pat_") {
        return Some("github_token");
    }
    if token.starts_with("AKIA") && token.len() >= 16 {
        return Some("aws_access_key");
    }
    if token.contains("-----BEGIN") || lower.contains("private_key") {
        return Some("private_key");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_fixtures_are_redacted() {
        for fixture in [
            "sk-12345678901234567890",
            "ghp_12345678901234567890",
            "AKIA1234567890123456",
            "password=hunter2",
            "Authorization:BearerToken",
            "AWS_SECRET_ACCESS_KEY=not-a-real-secret",
        ] {
            let (value, labels) = redact_secrets(fixture);
            assert_eq!(value, "[REDACTED]");
            assert!(!labels.is_empty());
        }
        let (value, labels) = redact_secrets("Authorization: Bearer not-a-real-token");
        assert_eq!(value, "Authorization: Bearer [REDACTED]");
        assert_eq!(labels, vec!["authorization_token"]);
        let (value, labels) =
            redact_secrets("-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----");
        assert_eq!(value, "[REDACTED PRIVATE KEY]");
        assert_eq!(labels, vec!["private_key"]);
    }

    #[test]
    fn forbidden_sources_are_rejected() {
        assert!(forbidden_source("/home/alice/.ssh/id_ed25519"));
        assert!(forbidden_source("/home/alice/.bash_history"));
        assert!(!forbidden_source("/home/alice/Documents/notes.txt"));
    }
}
