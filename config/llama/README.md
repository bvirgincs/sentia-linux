# Sentia local llama runtime configuration

`sentia-local.env` defines bounded defaults for local-only inference:

- private Unix socket only (`SENTIA_SOCKET_PATH`)
- single slot (`SENTIA_N_PARALLEL=1`)
- 8K context (`SENTIA_CTX_SIZE=8192`)
- bounded generation (`SENTIA_MAX_PREDICT=256`)
- idle sleep enabled (`SENTIA_SLEEP_IDLE_SECONDS=300`)

No API credentials are stored here.
