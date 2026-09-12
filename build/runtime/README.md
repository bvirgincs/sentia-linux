# Runtime packaging scripts

This directory provides reproducible runtime/model packaging helpers scoped to
Sentia runtime ownership.

## Scripts

- `inspect_llama_source.py` – validates required upstream build/runtime flags in
  llama.cpp `v0.4.0` commit `5266f24da75dc449bd56cbed7addb9c8e4a6a73e`.
- `fetch_granite_model.sh` – downloads pinned Granite model artifacts into
  `/home/ubuntu/sentia-linux/artifacts/downloads/granite-model`.
- `verify_granite_model.py` – checks model bytes/hash and `model.sig` bundle
  structure (does not claim cryptographic verification success).
- `prepare_llama_source.sh` – stages pinned llama.cpp source + Sentia overlays
  under `/home/ubuntu/sentia-linux/.build/runtime/llama-cpp/src`.
- `build_llama_deb.sh` – builds `sentia-llama-cpp` with heavy lock + `-j2`.
- `build_granite_model_deb.sh` – builds a binary `sentia-granite-model` `.deb`.
- `probe_llama_runtime.py` – health/readiness + generation probe over Unix socket.

## Toolchain expectations

Builds require host tools from the builder agent (no local apt in this worktree):
`dpkg-buildpackage`, `cmake`, `ninja`, `g++`, `make`, `pkg-config`, `flock`.
