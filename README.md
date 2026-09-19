# Sogni Client for Rust

An asynchronous Rust SDK for the Sogni Supernet and Sogni Intelligence APIs.
It follows the public wire contract of the TypeScript and Python clients,
while exposing Rust-native typed errors, streams, snapshots, and builders.
Version **5.50.4** implements the TypeScript **5.50.0** public contract through
[`452e789`](https://github.com/Sogni-AI/sogni-client/commit/452e78967a21ab80977c11f16517072d1836405a),
including hosted tool definitions from Sogni Protocol `1.0.0-alpha.42`.
It also includes authentication and streaming fixes from the
[Sogni-AI Rust fork](https://github.com/Sogni-AI/sogni-client-rs/commit/38b893c377c905816ae7a2365c0bc10a0dbc9a3f).
See [UPSTREAM.md](UPSTREAM.md) for attribution and synchronization policy.

The maintained crates.io package is [`sogni-client-by-morlz`](https://crates.io/crates/sogni-client-by-morlz).
It keeps the library target named `sogni_client`, so existing Rust `use`
paths remain unchanged. This repository no longer publishes new versions of
the previous `sogni-client` package.

## What is included

- API-key, JWT/refresh-token, cookie, and username/password authentication
- Reconnecting authenticated WebSocket transport with exponential backoff
- Image, video, and audio project submission, progress, results, cancellation,
  uploads, cost estimates, model metadata, LoRAs, and reconnect recovery
- SAM3 segmentation/cutouts, BiRefNet background removal, Pixal3D multi-view
  GLB reconstruction, GPT Image 2.5 editing, and worker result provenance
- Speech/voice cloning, FlashVSR video upscaling, FastH3 two-stage and
  audio-guide video, Seedance 2.5 exports, and reusable private uploads
- Socket-native streaming LLM chat, hosted chat completions, hosted tools, and
  durable chat runs with resumable SSE
- Durable creative workflows, cost confirmation, reseeding, SSE, and template
  CRUD/fork operations
- Account balances, wallet operations, rewards, subscriptions, replay records,
  leaderboards, and announcements
- Rustls TLS, bounded request timeouts, redacted credential diagnostics, no
  `unsafe`, and no blocking I/O on async paths

The model catalog is discovered from the service at runtime. Avoid hard-coding
the illustrative model identifiers used in examples if your application can
select from `projects.get_available_models()` instead.

## Requirements

- Rust 1.88 or newer
- Tokio runtime
- A Sogni account token pair or API key

The default `wallet` feature enables deterministic Sogni wallet derivation and
EIP-712 signing for username/password login, account creation, deposits, and
withdrawals. Disable default features if an application only uses API keys or
pre-issued tokens:

```toml
[dependencies]
sogni-client-by-morlz = { version = "5.50.4", default-features = false }
```

For development against the repository:

```toml
[dependencies]
sogni-client-by-morlz = { git = "https://github.com/morlz/sogni-client-rs", branch = "dev" }
```

## Authenticate with an API key

Do not embed keys in source code. Load them from a secret manager or the
environment.

```rust,no_run
use sogni_client::SogniClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = SogniClient::builder()
        .app_id("my-rust-service")
        .app_source("my-rust-service")
        .api_key(std::env::var("SOGNI_API_KEY")?)
        .build()
        .await?;

    // Keep one client for the lifetime of the process. It owns connection
    // pooling, token refresh, and the reconnecting socket task.
    client.close().await?;
    Ok(())
}
```

For an explicit SOCKS route, set `ClientBuilder::proxy_url(...)` with a
`socks5://host:port` URL (local DNS) or `socks5h://host:port` (proxy DNS).
It applies to REST, media transfer, and WebSocket connections without changing
global proxy settings. Proxy credentials are redacted from configuration debug
output. `strict_media_destinations(true)` restricts provider media transfers to
approved public HTTPS hosts, rejects redirects, and pins validated destination
addresses even through a proxy; TLS still verifies the original hostname.
Media requests honor `request_timeout`. Idempotent asset `PUT` retries transient
transport/service failures up to three times with the same URL and bytes inside
that original deadline. Input and authorization failures are not retried; this
does not broadly retry generation requests. The SDK can resend the original
project request after a server explicitly refuses admission during a restart;
an uncertain socket send still requires reconciliation.

For SSE event streams, `request_timeout` limits connection setup and idle reads,
not the total stream duration. Incoming data resets the read timeout. Drop the
stream to stop listening; use the API's cancel operation to cancel remote work.

The service requires the `sogni-client` WebSocket protocol-family identifier for
API-key authentication. The wire field uses that family with the actual compatible
version; HTTP User-Agent remains `sogni-client-rs/<version>`. A WebSocket upgrade
alone is not authentication: `is_socket_authenticated()` becomes true only after
the server's authenticated event. `abort()` synchronously stops the shared
client's transport and pending sends; it does not cancel remote projects. Scoped
execution owners should abort on lease loss and reconcile persisted project IDs.

Token authentication is available through
`SogniClient::builder().app_id("my-installation").tokens(token, refresh_token)`. For an interactive
username/password flow, create a token-auth client and call
`client.account.login(username, password)`.

Supply a stable `app_id` for WebSocket clients and persist it across restarts.
Blank IDs now fail locally when sockets are enabled, including deferred socket
startup; the builder no longer creates a random ID on every run. IDs need to be
unique among simultaneous connections for the same account. A second connection
using the same ID replaces the first.

For hosted chat, workflows, replay, announcements, or account REST APIs, use
`disable_socket(true)`. This mode never opens a socket and needs no app ID:

```rust,no_run
async fn rest_only() -> sogni_client::Result<()> {
    let client = sogni_client::SogniClient::builder()
        .api_key(std::env::var("SOGNI_API_KEY").expect("API key"))
        .disable_socket(true)
        .build().await?;
    client.close().await?;
    Ok(())
}
```

## Generate an image

```rust,no_run
use std::time::Duration;
use sogni_client::{Network, ProjectRequest, SogniClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = SogniClient::builder()
        .app_id("image-example")
        .api_key(std::env::var("SOGNI_API_KEY")?)
        .build()
        .await?;

    let project = client
        .projects
        .create(
            ProjectRequest::image("flux1-schnell-fp8", "A lighthouse in a winter storm")
                .network(Network::Fast)
                .steps(4)
                .guidance(1.0)
                .dimensions(1024, 1024),
        )
        .await?;

    let urls = project
        .wait_for_completion(Some(Duration::from_secs(30 * 60)))
        .await?;
    println!("{urls:#?}");
    client.close().await?;
    Ok(())
}
```

`wait_for_completion` never cancels the remote generation when its local
timeout expires. Call `project.cancel()` explicitly when cancellation is the
desired outcome. Cancellation waits for owner-scoped status to confirm
`finished=true`; a cancellation acknowledgment alone is not completion. A missing
or unavailable status leaves cancellation unconfirmed.

Durable backends can reserve and persist a UUID before calling
`projects.create_with_id(id, request)`, then use `projects.recover_project(id)`
after reconnect or restart with the same stable app ID. Explicit IDs do not
guarantee server idempotency: never resubmit after an uncertain send until the
original request has been reconciled. Missing projects remain `Unknown` when
the live registry is unavailable or malformed. `is_socket_connected()` exposes
transport continuity for applications that measure their own generation intervals.

`create_with_id_detailed` preserves an explicit `SubmissionPhase` and typed cause
without exposing private URLs in default error formatting. `Prepare` and
`AssetUpload` mean that this call did not send a generation request; `Send` may
have transmitted it. This says nothing about previous calls using the same ID.
Inspect typed cause/status/timeout fields instead of logging raw error text.

Recovery uses owner-scoped v2 project status: pending/queued/processing remain
active, and compact failed/canceled records terminate even when jobs are absent.
`get_status` exposes that response directly, while `get` retains its legacy v1
terminal-result contract. Inconsistent or unavailable status stays unknown; a
current 404 is not proof that a historical project never existed.

Completed generated-image jobs also support `job.enhance("light", overrides).await`;
the returned enhancement is tracked through `job.enhancement_project()`.
Segmentation masks/cutouts and 3D artifacts reject enhancement before download.

`job.preparation()` and `job.snapshot().preparation()` expose `JobPreparation`
while a worker downloads LoRAs, unloads a model, or loads the next model. Match
the enum variant before reading its fields. Model phases include a `Start` or
`End` step and optional elapsed seconds. Raw preparation data remains in the
snapshot's `extra` map, including future phases.

## Segment an image or reconstruct a 3D object

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest, Sam3ImagePrompt};

async fn segment(client: &sogni_client::SogniClient) -> sogni_client::Result<()> {
    let project = client.projects.create(
        ProjectRequest::image("sam3_image_segment_bf16", "")
            .asset(AssetRole::StartingImage, MediaSource::Path("source.png".into()))
            .sam3_prompt(Sam3ImagePrompt {
                text: Some("teapot".into()),
                apply_mask: Some(true),
                max_instances: Some(1),
                ..Default::default()
            })
    ).await?;
    let cutouts = project.wait_for_completion(None).await?;
    let _ = cutouts;
    Ok(())
}
```

SAM3 always requests one PNG and no previews, including in the local snapshot.
Omit `apply_mask` for a bare mask; set it to true for an RGBA source cutout.
`max_instances` keeps the highest-scoring 1–16 selections. Coordinates in
`Sam3PromptPoint` and `Sam3PromptBox` are normalized to the original source.
Prompts accept up to 32 points, 16 boxes, or 240 UTF-16 code units of text.
Text cannot be combined with points; point prompts allow at most one positive
box. Box labels default to positive; negative boxes exclude text-prompted
instances. Threshold defaults to 0.5. Multimask defaults to true for points;
text/box paths omit it and accept an explicit false. Invalid prompts fail
before an asset upload or generation submission.

BiRefNet uses image model `birefnet_image_background_removal_fp16`, an empty
prompt, and `AssetRole::StartingImage`. It also returns one PNG with no previews.
Its cutout control is the top-level `.param("applyMask", true)`; SAM3's control
belongs inside `Sam3ImagePrompt`.

Pixal3D uses image model `pixal3d_int8_i23d` with an empty prompt and a starting
image. `pixal3d_multiview_int8_i23d` additionally accepts any subset of
`AssetRole::Pixal3dLeftView`, `Pixal3dBackView`, and `Pixal3dRightView`. These
retain fixed upload slots 1, 2, and 3 when other views are omitted. Left/right
refer to the subject's own side. Both models reject generic context images.

```rust,no_run
use sogni_client::{AssetRole, MediaSource, Pixal3dGenerationOptions, ProjectRequest};

fn reconstruction() -> ProjectRequest {
    ProjectRequest::image("pixal3d_multiview_int8_i23d", "")
        .asset(AssetRole::StartingImage, MediaSource::Path("front.png".into()))
        .asset(AssetRole::Pixal3dBackView, MediaSource::Path("back.png".into()))
        .pixal3d_options(Pixal3dGenerationOptions {
            mesh_target_faces: Some(60_000),
            ..Default::default()
        })
}
```

`Pixal3dGenerationOptions` covers mesh/texture controls and shape resolution.
Only the single-view model accepts `Pixal3dTemplateVariant::I23dBirefnet`;
omitting the selector uses the worker default. Results are GLB 3D artifacts
with no previews; `job.media_type()` returns `"model"`. Live and recovered
downloads use `model/gltf-binary` even if stale catalog metadata says image.

`job.provenance()` and `JobSnapshot::provenance` expose optional `JobProvenance`
hashes and mask metadata from both live and persisted results. `Sam3Selection`
exposes optional confidence/bounds, coverage, and inclusion in the output mask.
Hashes are normalized to lowercase SHA-256 hex digests. Callers can set
`.world_generation_receipt(WorldGenerationReceiptRequest::TargetStill { ... })`
or `Transition { ... }`. The SDK validates receipt shape and hashes; applications
choose the recipe, and the service decides authorization and model eligibility.

## GPT Image 2.5, speech, and video utilities

GPT Image models include `gpt-image-2`, `gpt-image-2.5-sunburst`, and
`gpt-image-2.5-flare`. The 2.5 models add `xhigh`/`max` quality and transparent
PNG/WebP output. Set `gptImageQuality`, `gptImageBackground`, and
`gptImageOutputCompression` through `param`; compression is an integer 0–100
for JPEG/WebP. Quality `auto` is rejected. Editing accepts up to 16 ordered
context images. `AssetRole::GptImageMask` uploads the PNG alpha mask for the
first reference; `gptImageMaskUrl` accepts a URL or PNG data URI.

Speech uses `ProjectRequest::audio`, with the script in the prompt. Qwen3-TTS
models offer preset voices (`speaker`), voice design/delivery (`instruct`), or
voice cloning (`AssetRole::ReferenceAudio` and optional `referenceText`). Read
`get_model_options().raw` for each model's available controls: speech tiers do
not advertise music duration, tempo, or sampler settings they cannot consume.
The service validates which audio controls a model accepts.

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest};

fn speech() -> ProjectRequest {
    ProjectRequest::audio("qwen3_tts_1.7b_voice_clone_bf16", "Welcome to the studio.")
        .asset(AssetRole::ReferenceAudio, MediaSource::Path("voice.wav".into()))
        .param("referenceText", "The exact words spoken in the reference recording.")
}
```

FlashVSR is promptless and preserves the complete source video, its exact frame
rate, and its audio. Supply one `ReferenceVideo` and `upscaleResolution` 1080
or 1440. Frames/FPS/dimensions may be omitted for server probing; supplied timing
must match the source. No SDK clip-length cap is imposed. Optional
`detailPreference` (`stable`/`sharper`), `processingSpeed` (`stable`/`faster`),
and `seed` default to `stable`, `stable`, and 0; -1 requests a random seed.

```rust,no_run
use sogni_client::{AssetRole, FLASHVSR_VIDEO_UPSCALE_MODEL_ID, MediaSource, ProjectRequest};

fn upscale() -> ProjectRequest {
    ProjectRequest::video(FLASHVSR_VIDEO_UPSCALE_MODEL_ID, "")
        .asset(AssetRole::ReferenceVideo, MediaSource::Path("clip.mp4".into()))
        .param("upscaleResolution", 1440)
}
```

FastH3 uses `minimax-h3-fastvideo-int8_{mode}_turbo`, with `t2v`, `i2v`,
`flf2v`, `ia2v`, `flfa2v`, or `a2v`. Append `_2stage` for twice the canvas width
and height at the same timing. Send the normal H3 canvas, such as 672×384,
960×544, or 1344×768, and quote that model ID/canvas with `estimate_video_cost`.
The retired `outputScale` field and `_2stage_720p` request IDs are rejected.

Audio-guide modes require uploaded audio: `ia2v` also requires a first image,
`flfa2v` requires first and last images, and `a2v` takes audio alone. They use
4 steps, guidance 1, 24 FPS, and frames `124 + n*17` in 124–362. Output always
carries the uploaded audio; `audioStart` selects its offset. `audioDuration`,
LoRAs, and `generateAudio:false` are rejected. Use
`get_minimax_h3_frames_for_audio_duration(seconds)` for a covering frame count.

Seedance 2.5 accepts `.param("outputFormat", "mov")` and
`.param("returnLastFrame", true)`. Exported final frames are available through
`job.last_frame_url()` or refreshed with `job.get_last_frame_url().await`.

## Upload an input asset

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest};

fn request() -> ProjectRequest {
    ProjectRequest::image("qwen_image_edit_2511_fp8_lightning", "Make it cinematic")
        .asset(
            AssetRole::ContextImage(1),
            MediaSource::Path("input.png".into()),
        )
}
```

Adding an asset automatically enables its matching request flag. MiniMax H3
reference-to-video supports explicit numbered audio/video slots:

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest};

fn request() -> ProjectRequest {
    ProjectRequest::video(
        "minimax-h3-ref2va-fp8_r2v",
        "The subject walks into frame and speaks",
    )
        .duration(6.0)
        .asset(AssetRole::ReferenceImage, MediaSource::Path("subject.png".into()))
        .asset(
            AssetRole::ReferenceVideoSlot(1),
            MediaSource::Path("motion.mp4".into()),
        )
        .param("referenceVideoDurations", serde_json::json!([4.0]))
}
```

Numbered H3 audio/video slots must be contiguous from slot 1. The SDK validates
H3's frame lattice, dimensions, fixed FPS/steps/guidance, reference limits, and
duration hints before uploading. It also applies the current Wan 3, Seedance,
and HappyHorse task, asset, and duration constraints locally.

`client.projects.assets()` exposes `ReusableUploads` with `upload`, `list`,
`remove`, and `bind`. On supported accounts, normal project creation can reuse
matching verified private uploads. Continue passing the original files through
`asset`; a saved upload ID is not a replacement file parameter. Explicit upload
returns `SavedUpload`, and explicit binding takes `SavedUploadBinding` with a
project ID, asset type, and optional slot ID. Eligibility and verification are
server-owned; account changes invalidate cached availability.

When reusable uploads are unavailable, generation assets use the existing
presigned `PUT` flow. Failures after saved-asset transfer or verification begins
are reported. For independent uploads used by durable chat or workflows,
request a current v2 multipart form with `image_upload_post` or
`media_upload_post`, then call `upload_presigned`.

## Stream a chat completion

```rust,no_run
use futures_util::StreamExt;
use serde_json::json;
use sogni_client::SogniClient;

async fn run(client: &SogniClient) -> sogni_client::Result<()> {
    let mut stream = client
        .chat
        .stream_completion(&json!({
            "model": "qwen3.6-35b-a3b-gguf-iq4xs",
            "messages": [{"role": "user", "content": "Describe a nebula."}],
            "think": false
        }))
        .await?;

    while let Some(chunk) = stream.next().await {
        print!("{}", chunk?.content);
    }

    let final_result = stream.final_result();
    Ok(())
}
```

Use `create_completion` for a non-streaming socket call and
`create_hosted_completion` for the hosted OpenAI-compatible REST endpoint.
`client.chat.tools.all()` returns the 27 version-pinned hosted definitions,
including `generate_speech` and `upscale_video`. Hosted chat and durable runs
execute these server-side. `chat.execute_tool_call` directly runs six media
tools: `generate_image`, `edit_image`, `generate_video`, `sound_to_video`,
`video_to_video`, and `generate_music`.

Public routing helpers include `resolve_hosted_tool_model_selector`,
`is_edit_image_model`, `filter_video_models_by_workflow`, `get_video_defaults`,
and `select_backbone_model`. They recognize current GPT Image, FastH3,
audio-guide, and two-stage selectors. `BackboneModelOptions` applies compatible
requested models, then explicit preferences, then worker availability.

Socket chat keeps confirmed surviving streams across reconnection. A lost
request is surfaced as a structured retryable error rather than replaying an
LLM turn. Use `ChatError::retryable()` or `is_retryable_chat_error(&error)`
for `server_restarting` and `transport_lost` results.

## Durable chat and workflows

Durable operations accept only retrievable HTTP(S) media references. The client
rejects inline `data:` media before submission because it cannot survive a
disconnect and resume.

```rust,no_run
use futures_util::StreamExt;
use serde_json::json;

async fn run(client: &sogni_client::SogniClient) -> sogni_client::Result<()> {
    let run = client.chat.create_run(&json!({
        "messages": [{"role": "user", "content": "Create a launch campaign"}],
        "confirmCost": false
    })).await?;

    let run_id = run["id"].as_str().expect("server run id");
    let mut events = client.chat.stream_run_events(run_id, None).await?;
    while let Some(event) = events.next().await {
        println!("{:#?}", event?);
    }
    Ok(())
}
```

The workflow namespace provides `start`, `get`, `list`, `events`,
`stream_events`, `confirm_cost`, `resume`, `reseed`, and `cancel`; saved recipes
are available under `client.workflows.templates`. `WorkflowStart::safe_content_filter`
preserves an explicitly chosen false value on the request.

## Events and state

`SogniClient`, `ProjectsApi`, `Project`, `Job`, `ChatApi`, and `CurrentAccount`
offer broadcast receivers through `subscribe()`. Lagged receivers are explicit:
handle `tokio::sync::broadcast::error::RecvError::Lagged` and refresh a snapshot
when necessary. Project tracking automatically requests a durable recovery
snapshot after socket authentication and after an internal receiver lag.

Project and job handles are cheap clones backed by synchronized state. Use
`snapshot()` when an immutable, serializable view is needed.

When the server reassigns a render to another worker, the same `Job` handle and
logical `job_index()` remain valid while `job.id()` changes to the new attempt.
Late events from the old attempt do not regress the current result or progress.

Use `.defer_socket_start(true)` for authenticated HTTP-only catalogue access.
Unlike `disable_socket`, it preserves socket-hosted HTTP endpoints and starts
realtime I/O only when a socket command is sent. Give concurrent durable jobs
distinct, stable app IDs so catalogue and worker processes do not replace one
another's realtime sessions. Reuse the same job app ID during crash recovery.

## Errors

All fallible methods return `sogni_client::Result<T>`. Match the concrete error
variants when an application needs structured handling:

```rust,no_run
use sogni_client::{Error, is_subscription_limit_error};

fn inspect(error: Error) {
    if is_subscription_limit_error(&error) {
        // Prompt for the appropriate subscription change.
    } else if let Error::Api(api) = &error {
        eprintln!("HTTP {}: {}", api.status, api.message);
    }
}
```

The SDK does not log credentials or include them in `Debug` output. Server-side
authorization, billing, eligibility, and safety decisions remain authoritative.

## Compatibility and release checks

The crate version follows the upstream public client contract. Before release,
run:

```text
cargo fmt --all --check
cargo test --all-features
cargo test --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --all-features --no-deps
cargo package --allow-dirty
```

Public wire and hosted-routing fixtures can be refreshed from a built upstream
checkout with `scripts/update-generation-parity-fixtures.cjs` and
`.github/scripts/sync-chat-contract.cjs`. Keep generated data tied to the
recorded upstream revision. CI publishes a new manifest version to crates.io after a
successful default-branch push; see [RELEASING.md](RELEASING.md) for registry
credentials and release checks. Weekly synchronization runs as a Codex task on
Mondays at 06:00 UTC for both the TypeScript client and
[`Sogni-AI/sogni-client-rs`](https://github.com/Sogni-AI/sogni-client-rs), using
independent baselines in `.github/upstream-sync.json`. Applicable changes are
reviewed, tested, committed, and pushed before CI publishes a new crate version.
Echoed changes and fork-specific policy changes do not create empty merge
commits or releases; see [UPSTREAM.md](UPSTREAM.md).

See [Sogni documentation](https://docs.sogni.ai/),
[`sogni-client`](https://github.com/Sogni-AI/sogni-client), and
[`sogni-client-python`](https://github.com/Sogni-AI/sogni-client-python) for the
other supported clients and service-level concepts.

## License

ISC
