# Router tests

`run-unit.sh` runs the standalone crate unit tests while holding the shared
heavy-build lock and limiting Cargo to two jobs.

`test-actual-local-inference.sh` is an opt-in integration check. It only runs
when both the installed private llama socket and public local-broker socket
exist. It starts the real native router and sends a real chat request through
the broker to llama.cpp. Exit status 77 means the runtime was unavailable; a
skip is not evidence that local inference passed.

No test in this directory substitutes a mock response for the actual local
inference acceptance check.
