# Native provider workers

All remote providers are optional, disabled by default, and not live-tested.
The checked-in profiles describe API contracts; they do not select a model or
claim that a configured model exists. A router must require an explicit model
and perform privacy classification, minimization, preview, and consent before
creating a worker request.

## Security boundary

The worker accepts newline-delimited protocol v1 JSON on stdin and writes only
typed protocol events on stdout. It accepts text messages, a system instruction,
generation bounds, and a configured model. The schema has no files, URLs,
cookies, OAuth tokens, tools, shell commands, or arbitrary endpoint fields.
`ProviderWorkerIntegration` and `JsonLineProviderIntegration` expose this
versioned framing to the router without granting it direct adapter internals.

The API key is read once from inherited file descriptor 3. It is never accepted
in argv, environment variables, protocol JSON, or logs. The future launcher
must retrieve exactly one provider-scoped key from Secret Service (or another
reviewed per-user secret store), write it to a private pipe, and pass only the
read end as FD 3. Tests use an in-memory fixture secret and never inspect the
host credential store. The worker disables process dumps before reading FD 3;
the systemd policy also sets `LimitCORE=0`.

The HTTP client:

- uses Rustls with WebPKI roots and verified HTTPS;
- disables environment proxies and redirects;
- permits only the five compiled official destinations;
- performs no automatic retries;
- bounds connection, request, response-body, and output sizes;
- requests non-streaming, stateless text generation with no tools;
- maps errors to typed, redacted categories without returning vendor messages.

Cancelling a request aborts its task and drops the HTTP future. Killing the
worker also cancels the only active request. `store=false` disables API
conversation state where supported; it is not a promise that a vendor performs
no safety, abuse, or billing retention under its published terms.

## Current reviewed contracts

Reviewed against official documentation on 2026-09-12:

| Provider | Implemented API contract | Privacy/tool choices |
|---|---|---|
| OpenAI | `POST https://api.openai.com/v1/responses` | `store=false`, `stream=false`, empty tools, `tool_choice=none` |
| Anthropic | `POST https://api.anthropic.com/v1/messages`, `anthropic-version: 2023-06-01` | non-streaming text messages; no tools field |
| Google | `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent` | stateless `generateContent`; no tools field |
| xAI | `POST https://api.x.ai/v1/responses` | `store=false`, `stream=false`, search off, empty tools, `tool_choice=none` |
| DeepSeek | `POST https://api.deepseek.com/chat/completions` | `stream=false`; no tools field |

Google recommends the newer Interactions API for new projects, while stating
that `generateContent` remains fully supported. This worker deliberately uses
`generateContent`: stateless multi-turn Interactions requests must replay typed
model steps (including thought signatures), which the sanitized text-only
protocol intentionally does not carry.

Official references:

- OpenAI Responses:
  <https://developers.openai.com/api/reference/resources/responses>
  and <https://developers.openai.com/api/docs/guides/migrate-to-responses>
- OpenAI documented OpenAPI:
  <https://app.stainless.com/api/spec/documented/openai/openapi.documented.yml>
- Anthropic Messages, errors, and versioning:
  <https://platform.claude.com/docs/en/api/messages>,
  <https://platform.claude.com/docs/en/api/errors>,
  <https://platform.claude.com/docs/en/api/versioning>
- Gemini migration and `generateContent`:
  <https://ai.google.dev/gemini-api/docs/migrate-to-interactions>
- xAI Responses:
  <https://docs.x.ai/developers/rest-api-reference/inference/responses>
  and <https://docs.x.ai/developers/model-capabilities/text/generate-text>
- DeepSeek quick start and error codes:
  <https://api-docs.deepseek.com/> and
  <https://api-docs.deepseek.com/quick_start/error_codes>

No model name from those pages is a default in Sentia.

## Billing and CLI eligibility

Every profile labels API traffic as **separately billed API usage**. A consumer
subscription is **not an API billing entitlement**.

No consumer website scraping, browser cookies, private APIs, OAuth extraction,
or subscription reuse is implemented. Claude Code remains a conditional
official-client candidate only after explicit terms review and an authorized
all-tools-off test. Codex account login, Antigravity consumer OAuth, Grok Build
subscriptions, and the DeepSeek preview harness are not provider credentials
or generic autonomous CLI wrappers.

## Enablement status

The systemd policy template is intentionally fail-closed because the router's
descriptor-based transient activation has not yet been integrated and tested
on Debian 13. No qualification marker is shipped. Do not create that marker
until an installed-system test proves:

1. stdin/stdout are an unnamed private socket pair owned by the router;
2. only the selected provider credential is attached as FD 3;
3. `ProtectHome` and runtime path masking are enforced;
4. the worker cannot create Unix sockets or reach router/control sockets;
5. cancellation, timeout, and process death are observed correctly.

Even after boundary qualification, each provider remains disabled until the
account owner configures a model, stores an API key, accepts separate API
billing, and authorizes a live conformance request.
