# Providers

## Status

The provider findings in the approved plan are documentation-only at this
checkpoint. No live provider login, token capture, or authenticated request has
been run here.

## Initial posture

| Ecosystem | Initial implementation posture | Live verification |
| --- | --- | --- |
| OpenAI | Direct API adapter. | not yet verified |
| Anthropic | Direct API adapter; official-client embedding is conditional on safe all-tools-off behavior. | not yet verified |
| Google | Direct Gemini API adapter; do not wrap consumer OAuth under the current restrictions. | not yet verified |
| xAI | Direct API adapter; hold subscription routing until entitlement and genuine no-tool operation are verified. | not yet verified |
| DeepSeek | Direct API adapter; do not make the preview harness a production dependency. | not yet verified |

## Boundary notes

- No account identifiers, private hostnames, or credentials are recorded here.
- Remote adapters remain disabled or experimental until a real authorized
  request proves them safe and functional.
- These findings remain docs-only until implementation evidence exists.
