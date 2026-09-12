# sentia-granite-model packaging

Pinned model payload for Sentia local runtime:

- repository: `ibm-granite/granite-4.2-3b-GGUF`
- revision: `c40945d71cd90f249a56985e8155551a9188dc30`
- file: `granite-4.2-3b-Q4_K_M.gguf`
- bytes: `2244011552`
- sha256: `e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`

Model weights are downloaded to ignored artifacts under:
`/home/ubuntu/sentia-linux/artifacts/downloads/granite-model`.

`model.sig` is included and structurally validated by script. Cryptographic
verification requires the official IBM/Sigstore flow.
