# sentia-llama-cpp packaging overlay

This overlay packages pinned upstream `llama.cpp`:

- upstream tag: `v0.4.0`
- upstream commit: `5266f24da75dc449bd56cbed7addb9c8e4a6a73e`
- upstream license: MIT

Build design goals:

- portable CPU baseline (`GGML_NATIVE=OFF`)
- dynamic backend loader (`GGML_BACKEND_DL=ON`)
- CPU all variants (`GGML_CPU_ALL_VARIANTS=ON`)
- no GPU runtime backends
- server build with Web UI disabled
- local Unix socket service configuration and readiness probe integration

Use `build/runtime/build_llama_deb.sh` to stage upstream source and run the
package build.
