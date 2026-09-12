# Provider contract fixtures

These JSON files exercise request serialization, documented success envelopes,
typed error mapping, and secret redaction. They use the explicit placeholder
`configured-model`; they do not assert that any model is available.

The tests never open a listener, read host credentials, or contact a provider.
Provider state remains disabled and `live_tested = false` until an account owner
authorizes a separately billed API request.

Run `./tests/providers/run-offline.sh` from any directory. It holds the shared
heavy-job lock, limits Cargo to two jobs, runs the Rust fixture tests, checks the
capability labels, and sends only an invalid-model command that is rejected
before any network request can be built.
