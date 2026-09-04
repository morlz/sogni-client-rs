# Sogni Rust SDK examples

These examples are runnable ports of the TypeScript and Python SDK examples. They cover image,
video, audio, chat, hosted tools, durable workflows, recovery, and a small Axum application.

## Safety and credentials

Generation is paid network activity. The generation examples default to a credential-free dry
run and print the request they would submit. Pass `--execute` only after reviewing it. Examples
that fetch an estimate ask for confirmation; pass `--yes` for intentional unattended runs.

Show help without credentials:

```console
cargo run --example workflow_text_to_image -- --help
```

Use an API key (recommended):

```text
SOGNI_API_KEY=your_api_key
SOGNI_TOKEN_TYPE=spark
SOGNI_BILLING_MODE=auto
```

The helpers read process environment variables first, then `examples/.env`, then `.env`. They also
support `SOGNI_USERNAME` plus `SOGNI_PASSWORD`. Username/password authentication requires the
default `wallet` feature; builds using `--no-default-features` should use `SOGNI_API_KEY`.

Optional development endpoint overrides are `SOGNI_REST_ENDPOINT`, `SOGNI_SOCKET_ENDPOINT`, and
`SOGNI_TESTNET=true`. TLS certificate verification remains enabled.

## Quick start

Inspect a text-to-image request:

```console
cargo run --example workflow_text_to_image -- \
  "A serene mountain lake at sunrise" --model z-turbo --no-interactive
```

Estimate, confirm, generate, and download it:

```console
cargo run --example workflow_text_to_image -- \
  "A serene mountain lake at sunrise" --model z-turbo --no-interactive --execute
```

Reference-based identity edit:

```console
cargo run --example workflow_image_edit -- \
  "cinematic editorial portrait" \
  --context examples/test-assets/placeholder.jpg \
  --model krea-identity-edit --no-interactive --execute
```

Upscale an image to a 3840-pixel longest edge:

```console
cargo run --example workflow_upscale_image -- \
  --image examples/test-assets/placeholder.jpg --target 3840 --no-interactive --execute
```

Use `cargo run --example NAME -- --help` for every example's authoritative options.

## Example catalog

### Fundamentals

| Rust target | Demonstrates |
| --- | --- |
| `image_generation` | Concise image request, estimate, progress, and download |
| `promise_based` | Submit/await/download flow with live model selection |
| `event_driven` | Project and job event streams plus completion waiting |
| `chat_stream` | Minimal streaming chat consumption |
| `workflow_project_resume` | Reconnect, recover, and resume project tracking |

### Image workflows

| Rust target | Demonstrates |
| --- | --- |
| `workflow_text_to_image` | Interactive or scripted T2I, model defaults, img2img, LoRA, previews, estimate, download |
| `workflow_image_edit` | Qwen general editing or Krea identity preservation with numbered context images |
| `krea_identity_edit` | Compact Python-client-compatible identity edit |
| `workflow_upscale_image` | Deterministic RTX VSR scaling, aspect preservation, 8-pixel alignment, 16K cap |
| `workflow_multiple_angles` | Qwen Multiple Angles LoRA and its 96 pose combinations |
| `workflow_krea2_lora_stack` | Ordered bipolar Krea 2 LoRA stacks and reverse-order comparison |
| `lora_examples_strip` | Fixed-seed LoRA strength strip on a pinned worker |
| `lora_order_test` | Every permutation of an ordered LoRA stack |
| `benchmark_text_to_image` | Warmup/measured min/default/max step benchmarks and JSON timing model |

Choose `krea-identity-edit` when a person or character must remain recognizable. Use Qwen image
edit for general transformations, text editing, multi-person work, or combining up to three
references. A higher-step general editor is not a substitute for the identity model.

### Video workflows

| Rust target | Demonstrates |
| --- | --- |
| `workflow_text_to_video` | Text-to-video model families, duration/frame rules, estimate, progress, metadata |
| `workflow_image_to_video` | First/last-frame animation from local images |
| `workflow_batch_i2v` | Batch image-to-video processing |
| `workflow_sound_to_video` | Image plus audio conditioning |
| `workflow_video_to_video` | Motion transfer, character replacement, and V2V controls |
| `workflow_minimax_h3_video` | MiniMax H3 and H3 Turbo modes |
| `workflow_partner_seedance_video` | Seedance partner API modes and multimodal references |
| `workflow_seedance_2_5_r2v` | Seedance 2.5 reference/edit/extend contract |

Video examples use model-specific frame behavior. LTX 2.3 generates at the selected FPS with
`duration * fps + 1` frames snapped to `1 + n*8`. WAN 2.2 generates internally at 16 FPS; a 32 FPS
selection is post-render interpolation. Partner models use their provider-owned frame rates.

Several video examples inspect downloaded output using `ffprobe`. Install FFmpeg and ensure
`ffprobe` is on `PATH` when metadata validation is requested.

### Chat and creative intelligence

| Rust target | Demonstrates |
| --- | --- |
| `workflow_text_chat` | Non-streaming socket-native chat |
| `workflow_text_chat_streaming` | Incremental chat chunks |
| `workflow_text_chat_multi_turn` | Conversation history across turns |
| `workflow_text_chat_tool_calling` | Custom function tools and returned tool calls |
| `workflow_text_chat_vision` | PNG/JPEG vision preprocessing and multimodal content |
| `workflow_text_chat_sogni_tools` | Sogni hosted media tools from chat |
| `workflow_direct_creative_tool` | Direct synchronous composition tools |
| `workflow_creative_agent_cli` | Natural-language creative-agent command line |
| `workflow_creative_agent_tools` | Hosted tool planning/execution |
| `workflow_creative_agent_workflows` | Durable creative workflows, event streaming, and lifecycle operations |
| `workflow_text_to_music` | ACE-Step 1.5 audio generation |

### Axum web application

`http_server` ports the Express demo to Axum. It binds to loopback by default, limits JSON bodies
and concurrent generations, exposes `/healthz`, and requires the browser to estimate and confirm
each paid request before `/api/generate` accepts it.

```console
cargo run --example http_server -- --execute
```

Then open `http://127.0.0.1:3000`. A non-loopback bind additionally requires `--allow-remote`;
add application authentication and HTTPS before exposing the example to untrusted networks.

## Test media and output

`examples/test-assets` contains the same small image, audio, and video fixtures used by the
TypeScript examples. Generated media defaults beneath `examples/output` (some compact basic
examples use their historically named output directory). Downloads stream to a temporary file,
then rename atomically, and never overwrite an existing result.

Server-hosted result URLs are temporary, so download outputs you need to keep.

## Validation

Compile examples with the crate's default features:

```console
cargo check --examples
```

Verify API-key-only builds:

```console
cargo check --examples --no-default-features
```

## Troubleshooting

- `model ... is not currently available`: choose a model returned by
  `projects.wait_for_models()` or retry when workers are available.
- Credential error: set `SOGNI_API_KEY`, or enable `wallet` and set username/password.
- Confirmation error in CI: add `--yes` only when the run is intentionally authorized to spend.
- Media input error: pass a real local file; paths are uploaded as bytes by the SDK.
- `ffprobe` error: install FFmpeg or omit the metadata-checking option where supported.
- Insufficient balance: reduce count, duration, dimensions, or steps, or select the appropriate
  billing mode.

See the [Sogni documentation](https://docs.sogni.ai/), the
[SDK API documentation](https://sdk-docs.sogni.ai), and the repository's root README for the
complete Rust API.
