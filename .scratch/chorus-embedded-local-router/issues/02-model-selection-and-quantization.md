# Local model weight selection and GGUF quantization

Type: research
Status: resolved
Blocked by:

## Question

Which candidate model (<1 GB Q4_K_M GGUF) achieves the highest routing accuracy (JSON schema conformity) and conversation compaction quality for bilingual Thai/English tasks on Apple Silicon?

Specifically evaluate:
1. Candidate weights: `qwen2.5-coder:1.5b-instruct-q4_k_m` (~980 MB), `qwen2.5:0.5b-instruct-q4_k_m` (~390 MB), `openthai-systemone:0.8b` (~550 MB), and `typhoon2:1.5b-instruct-q4_k_m`.
2. Reasoning depth vs size: Can a 0.5B model reliably output structured JSON decision parameters (`complexity_score`, `recommended_model`, `recommended_agent`) without hallucination or malformed JSON?
3. Compaction capability: Can the model summarize a 4,000-token conversation history into key state points without losing critical codebase paths or instruction context?
4. Hugging Face repository hosting URL, official quantizer license, and exact file size in bytes for download calculation.

## Answer

- **Routing Champion**: `openthai-systemone:0.8b-q4_k_m` (529 MB, Apache 2.0) achieves highest accuracy (50.0%, 17/34) and lowest latency (380 ms p95 on Metal).
- **Dual-Use Invalidation**: `openthai-systemone:0.8b` is a slot classifier without an LM head; it cannot generate text or summarize conversations. 0.5B generative models fail both structured JSON routing (<10/34, hallucination, no epistemic abstention) and 4,000-token compaction (attention dilution, dropped paths).
- **Compaction Strategy**: Retain model-free `compaction::Extractive` as the 0-byte, 0-ms default preserving exact paths and diffs. If neural compaction is required, use `qwen2.5-coder:1.5b-instruct` (Q3_K_M: 893 MB / Q4_K_M: 1,117 MB).
- **Download URL**: `https://huggingface.co/iapp/OpenThai-SystemOne-GGUF/resolve/main/OpenThai-SystemOne-v0.3-Q4_K_M.gguf` (529,313,920 bytes, Apache 2.0).

Detailed findings: [02-model-selection-and-quantization.md](../research/02-model-selection-and-quantization.md).
