# Router model comparison

`cases.json` is a small, manually labeled Thai/English smoke set for Chorus's
current three-tier routing policy. Each `expected` value is the agent/model
effort tuple the policy should choose from the fixed live candidate grid;
`manual` means the request is intentionally too ambiguous to route safely.
The set is useful for catching obvious regressions, not for claiming general
model superiority. It contains no private user prompts.

Run with a local Ollama server and installed models:

```bash
cargo run -q -p chorus-cli --bin router_eval -- evals/router/cases.json \
  qwen2.5:3b qwen3:1.7b tev1:0.8b chorus-openthai-systemone:0.8b
```

The harness warms each model, then reports exact route matches, manual
selection count, Thai/English matches, and the nearest-rank p95 wall time.
Every call includes capability discovery, candidate validation, and inference.
Run models sequentially on the same machine with no competing inference load.
Use `--decision-threshold 0.45` after the case path to test a stricter
System One abstention gate; the production default is 0.35.

As of 2026-10-05, the published `iapp/openthai-systemone:0.8b` and matching
`hf.co/iapp/OpenThai-SystemOne-Ollama:Q8_0` tags did not advertise the
`decision` capability on Ollama 0.35.1. A local alias using the same weights
was required to exercise the System One endpoint:

```bash
curl http://127.0.0.1:11434/api/create -H 'Content-Type: application/json' -d \
  '{"model":"chorus-openthai-systemone:0.8b","from":"iapp/openthai-systemone:0.8b","capabilities":["decision"],"stream":false}'
```

Verify the alias through `/api/show` before routing. Keep the original tag
available until the publisher fixes its capability metadata.
