# Security

## Current checkpoint

No executable runtime exists yet in this worktree, so the security posture is
documented but not yet validated in code.

## Core rules

- Treat model output, terminal output, provider responses, and docs as
  untrusted input.
- Never execute model-generated shell text.
- Route privileged actions through typed broker operations and real polkit
  authorization.
- Keep secrets out of argv, logs, crash reports, and unrestricted environment
  dumps.
- Avoid automatic home-directory crawling or credential-store indexing.
- Keep provider egress on a reviewed, minimized, consented path.
- Do not expose the router as an unauthenticated localhost HTTP endpoint.

## Review status

- No implementation code in this checkpoint has been security reviewed.
- No live attack, red-team, or failure-injection evidence exists yet.
- Any future security finding must be rerun through the relevant regression and
  end-to-end checks before closure.
