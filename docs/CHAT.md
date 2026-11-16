# Local Chat

Fluent's chat is a separate capability from translation. It runs a quantized
instruction-tuned causal language model locally, in its own engine, worker,
model directory and GUI view. Translation models are never used as chat models.

## 1. Models

| Model | Parameters | Quantization | Download | Context | Licence |
|---|---:|---|---:|---:|---|
| Qwen2.5 1.5B Instruct | 1.5B | Q4_K_M GGUF | 1.12 GB | 4096 used (32768 upstream) | Apache-2.0 |
| Qwen2.5 0.5B Instruct | 0.5B | Q4_0 GGUF | 0.44 GB | 4096 used (32768 upstream) | Apache-2.0 |

The 1.5B model is the default: it is noticeably more coherent than the 0.5B
model while still fitting the CPU baseline. The 0.5B model is the fast option
for low-end systems.

Upstream:

- `Qwen/Qwen2.5-1.5B-Instruct-GGUF` revision
  `91cad51170dc346986eccefdc2dd33a9da36ead9`
- `Qwen/Qwen2.5-1.5B-Instruct` revision
  `989aa7980e4cf806f80c7fef2b1adb7bc71aa306`
- `Qwen/Qwen2.5-0.5B-Instruct-GGUF` revision
  `9217f5db79a29953eb74d5343926648285ec7e67`
- `Qwen/Qwen2.5-0.5B-Instruct` revision
  `7ae557604adf67be50417f59c2c2f167def9a775`

The GGUF and tokenizer files are pinned by size and SHA-256 in
`crates/fluent-core/src/model/chat_registry.rs`. Fluent does not bundle or
redistribute the weights.

## 2. Architecture

```text
GUI Chat view
    │  messages, model id, ChatOptions
    ▼
ChatWorker (separate thread)
    │
    ▼
fluent_core::ChatEngine
    ├── candle quantized_qwen2 (GGUF, CPU)
    ├── HF tokenizer.json
    ├── Qwen2.5 chat template
    └── greedy sampler + repetition penalty
```

- `ChatEngine::load` finds the single `.gguf` in the model directory and the
  `tokenizer.json`, then validates the `<|im_start|>`, `<|im_end|>` and
  `<|endoftext|>` tokens.
- `ChatEngine::reply` builds the system/user/assistant transcript, encodes it,
  prefills, and generates one token at a time. Each generated token is decoded
  and streamed as a text delta.
- The KV cache is reset at the start of every reply (`index_pos = 0`), so a
  cancelled request cannot corrupt the next one.
- Chat and translation use separate workers. Switching to Chat unloads the
  translation model; switching to Translate sends an `Unload` to the chat
  worker. Fluent does not keep two heavyweight models resident by default.

### Prompt format

```text
<|im_start|>system
<default instruction><|im_end|>
<|im_start|>user
…<|im_end|>
<|im_start|>assistant
```

A leading system message from the caller overrides the default instruction.
The default is a short "answer clearly and concisely; say when unsure" prompt.

### Sampling

Greedy decoding with a repetition penalty of 1.1 over the last 96 generated
tokens. This is deterministic for a fixed prompt, model and build, and avoids
the most common short-model repetition loops. There is no temperature/top-p
control in this release.

### Context and truncation

- Prompt + generated tokens are capped at **4096**.
- At most **512** tokens are generated per reply.
- When a conversation exceeds the prompt budget, the oldest user/assistant
  turns are dropped first. If a single message is still too long, its beginning
  (where the instruction usually is) is kept and the assistant suffix is
  appended. The UI shows "Older conversation turns were omitted…".
- Reaching the 512-token output ceiling is reported separately.

## 3. Persistence

**Chat history is session-only.** Closing Fluent clears the conversation. No
chat text is written to `settings.json`, the model directory or any other file.
This is a deliberate privacy choice: the model files are user-supplied and the
conversation is not needed for translation history. "New chat" clears the
session immediately.

The selected chat model id, window theme and model paths are persisted, but
not the messages.

## 4. Download and lifecycle

Chat models appear in **Models → Chat models** with the same native installer
as translation models:

- pinned repository and revision;
- exact file sizes and SHA-256;
- resumable `.part` downloads;
- disk-space check before starting;
- cancellation and incomplete-state handling;
- atomic promotion only after verification;
- safe removal confined to the chat model directory.

The Chat view shows the selected model, installed state, download guidance and
a picker for the two chat models. Selecting a different chat model starts a new
conversation and unloads the previous engine.

Chat model selection is independent of translation model selection. Changing
one never changes the other.

The status pill distinguishes **not installed**, **installed but not loaded**
and **ready**. Switching to Translate unloads the chat engine while the model
remains installed on disk; returning to Chat shows *Chat model not loaded*
until a message starts the worker and emits `Loaded`.

### Cycle 19 GUI validation (Linux x86-64, CPU)

The 1.5B model was downloaded through the GUI into a fresh `/download` root:

- 1,124,359,686 bytes total: GGUF 1,117,320,736 B
  (`6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`),
  `tokenizer.json` 7,031,645 B
  (`c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539`)
  and `tokenizer_config.json` 7,305 B
  (`5b5d4f65d0acd3b2d56a35b56d374a36cbc1c8fa5cf3b3febbbfabf22f359583`).
- The progress view reported about 2 MB/s with an ETA. The run took about 12
  minutes end to end, including startup, navigation and SHA-256 verification;
  the row then showed **Installed** and `verify_chat_model` reported
  `Installed, verified=true, missing=0, bad=0`.
- Cancelling at 37.6 MB kept the `.part` file and left the row **Not
  installed** with a Download action. Starting again resumed the same file:
  the `.part` grew from 37,625,448 to 69,889,816 bytes in 15 seconds instead of
  restarting at zero, and the completed model passed the pinned SHA-256
  verification. The `resumes_a_partial_download_and_verifies_the_hash` unit
  test covers the same path independently.
- Loading, an English question, a French follow-up, switching to Translate and
  returning to Chat all behaved correctly. Switching away unloaded the engine
  (RSS ~2.03 GB to ~0.96 GB); returning showed *Chat model not loaded* until
  the next message reloaded the model and produced an answer.

## 5. Measured performance

Canonical host: Intel Core i5-11400H (6C/12T), 15 GiB RAM, CPU only, Nobara
Linux 44. Reproduce with:

```bash
cargo run --release -p fluent-core --example chat_eval -- \
  --model ~/.local/share/fluent/models/qwen2.5-1.5b-instruct \
  --id qwen2.5-1.5b-instruct --out /tmp/chat.json
```

| Model | Load | RSS after load | Peak RSS | TTFT (short prompt) | Decode rate |
|---|---:|---:|---:|---:|---:|
| Qwen2.5 1.5B Q4_K_M | 1.9 s | 1.95 GB | 2.02 GB | 3.3–4.9 s | 4–7 tok/s |
| Qwen2.5 0.5B Q4_0 | 1.0 s | 0.94 GB | 0.96 GB | 2.6–3.2 s | 5–8 tok/s |

End-to-end throughput is lower for short replies because prefill dominates: a
7-token answer measured 1.2–1.9 tok/s including the prompt. A ~300-token
follow-up takes ~13–17 s to prefill on this CPU. The 1.5B model is about 4×
slower than the 0.5B model for the same prompt.

The `chat_eval` report records prompt/generated tokens, TTFT, duration,
tokens/s, streaming equality, context truncation and output-limit flags.

Switching views drops the other engine. With the default allocator, RSS can
remain high after unload because freed arenas are retained (chat unload stayed
near 0.96 GB in one GUI run). Repeating the sequence with
`MALLOC_TRIM_THRESHOLD_=0 MALLOC_MMAP_THRESHOLD_=65536` showed RSS falling from
~2.05 GB to ~0.14 GB after chat unload and from ~1.74 GB to ~0.18 GB after
translation unload, which confirms the engine is actually dropped rather than
kept resident.

## 6. Known limitations

- Both models are small. They can be factually wrong, especially in French and
  on detailed science questions. The 0.5B model is noticeably weaker.
- No tool/function calling, vision, RAG or web access.
- No temperature/top-p controls; greedy decoding is deterministic but can be
  repetitive on open-ended prompts.
- CPU only in this release. CUDA/Metal chat inference is not validated.
- Long conversations are prefill-bound; 4096 tokens is the practical limit.
- Chat history is intentionally not persisted.
- Chat is not a replacement for the translation workflow, and translation
  models are not exposed as chat models.

## 7. Tests

- `crates/fluent-core/src/chat.rs`: prompt construction and system override.
- `crates/fluent-core/tests/chat.rs` (gated by `FLUENT_CHAT_MODEL`): real
  load, reply, determinism, follow-up context, cancellation during streaming,
  reload and context truncation.
- `crates/fluent-core/src/model/chat_registry.rs`: unique ids/dirs, sizes,
  hashes, revisions and default selection.
- `model_manager.rs`: chat manifests use the same installer/removal
  boundaries.
- GUI: chat view, model picker, streaming, cancellation, Model Manager section
  and translation/chat isolation.
