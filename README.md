# Sogni Client for Rust

An asynchronous Rust SDK for the Sogni Supernet and Sogni Intelligence APIs.
It follows the public wire contract of the TypeScript and Python clients,
while exposing Rust-native typed errors, streams, snapshots, and builders.
Version **5.60.7** implements the TypeScript **5.60.7** public contract through
[`1683bf3`](https://github.com/Sogni-AI/sogni-client/commit/1683bf33a8377a968aa3682021560ff729cb1a2e),
including hosted tool definitions from Sogni Protocol `1.0.0-alpha.47`.
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
  queue explanations, durable result lookup, uploads, estimates, and recovery
- SAM3 segmentation/cutouts, BiRefNet background removal, Pixal3D multi-view
  GLB reconstruction, GPT Image 2.5 editing, and worker result provenance
- Speech/voice cloning, FlashVSR video upscaling, MiniMax H3 keyframes,
  two-stage/audio-guide video with LoRAs, Seedance exports, and private uploads
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
sogni-client-by-morlz = { version = "5.60.7", default-features = false }
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

Logout or an account switch ends pending REST, SSE, chat, and project operations
from the previous account. Old `Project` and `Job` handles cannot start operations
for the new account; existing waiters receive a session-ended error. This does
not cancel remote generation. Refreshing tokens for the same wallet preserves
ongoing work.

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

Image requests accept `.embed_prompt_metadata(false)` to omit generation
prompt/settings from worker image metadata; omission leaves the worker default
enabled. `startingImageStrength` is source-image influence from 0 to 1: 0 requests
full denoising and 1 preserves the input. Omission or null uses 0.5. Explicit
zero values for strength, guidance, seed, and preview count remain on the wire.

For Stable Diffusion ControlNet, set `controlNet.preprocess` to `true` when
`AssetRole::ControlNetImage` is an ordinary photo: the worker builds the map
for the selected ControlNet name. Omitted or `false` uses the uploaded map
as supplied, preserving existing requests. The option must be a boolean;
InstantID, inpaint, and instrp2p use their image as supplied.

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

After a connection drops, recovery may resend a successfully written request
once if owner status and the live registry both confirm its absence and no
server event or lookup has ever acknowledged it. The resend keeps the same ID
and payload. Failed or uncertain writes and requests from an old account session
do not qualify.

Completed generated-image jobs also support `job.enhance("light", overrides).await`;
the returned enhancement is tracked through `job.enhancement_project()`.
Enhancement uses Krea 2 Turbo (`krea2_turbo_fp8_scaled`) at 8 steps on Fast.
`light`, `medium`, and `heavy` apply denoising strengths 0.15, 0.35, and 0.49.
The source job's seed, including zero, is retained. Explicit dimensions and
named size presets are resolved from the parent model and preserved; an unknown
preset fails before downloading the source or creating an enhancement.
Quote the same canvas with
`projects.estimate_enhancement_cost_with_size(strength, token_type, width, height)`;
the existing `estimate_enhancement_cost(strength, token_type)` uses default size.
Segmentation masks/cutouts and 3D artifacts reject enhancement before download.

`job.preparation()` and `job.snapshot().preparation()` expose `JobPreparation`
while a worker downloads LoRAs, unloads a model, or loads the next model. Match
the enum variant before reading its fields. Model phases include a `Start` or
`End` step and optional elapsed seconds. Raw preparation data remains in the
snapshot's `extra` map, including future phases.

## Look up results and queue state

`projects.get_result(id, None)` reads an account-owned project from durable
status and returns `ProjectResult` with current status, jobs, and available
completed-result URLs. It works after a process restart or socket recovery
expiry, without creating a project or changing local tracking.

Use `get_result` and `get_status` for individual reads. Wait on a tracked
project through `wait_for_completion` and socket events; repeated REST reads
consume the account's request limit and result reads may also mint one URL per
completed job. Honor `ApiError::retry_after()` when the server returns 429.

```rust,no_run
use sogni_client::{ListRecentProjectsOptions, SogniClient};

async fn recent_results(client: &SogniClient) -> sogni_client::Result<()> {
    let recent = client.projects.list_recent(Some(ListRecentProjectsOptions {
        limit: Some(20),
        ..Default::default()
    })).await?;
    for project in recent {
        let result = client.projects.get_result(&project.id, None).await?;
        println!("{}: {} jobs", result.id, result.jobs.len());
    }
    Ok(())
}
```

`list_recent` defaults to the last 24 hours, accepts `since` in epoch
milliseconds and an optional `app_source`, and clamps history to seven days.
Its `limit` is 1–100 renders (default 50), grouped into projects newest first.
It does not mint download URLs. Result URLs expire; call `get_result` again for
fresh URLs. A result job's `url_unavailable` explains withheld media, unknown
media kind, or a failed URL lookup. `GetProjectResultOptions.kind` supplies a
fallback `ResultMediaKind` only when stored evidence and model metadata cannot
identify the media. Unknown media is never assumed to be an image.

Sogni signed result links are valid for 48 hours. Download outputs you need to
keep, or request a fresh URL later. Treat upload/download links as opaque:
their hostname may use S3 Transfer Acceleration or Sogni's R2 storage, and their
signed query must remain intact.

Queue explanations are available through `project.waiting_reason()`,
`project.job_waiting_reasons()`, `job.waiting_reason()`, and their snapshots.
The `queueChanged` event from `projects.subscribe()` carries
`ProjectQueueChanged`. Display `WaitingReason.message` as plain text. A batch
can keep queued jobs while another job runs; missing explanations on older
servers do not imply an error or promise immediate processing.
The socket subscribes to `projectQueue` by default. Opt out with
`.socket_event_subscription("projectQueue", false)` or an explicit false in
`client.set_socket_event_subscriptions(...)`.

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

Music uses `ProjectRequest::audio("minimax_music3", prompt)` or one of the
ACE-Step models. MiniMax Music 3 is the default for `generate_music`: duration
10–300 seconds (default 60) is a ceiling, so the track can resolve earlier;
steps are 10–100 (default 30), guidance 1–5 (default 1.7), and `promptStrength`
0–10 (default 1.7). Put tempo and key in the prompt. Lyrics use plain section
tags such as `[Verse]` and `[Chorus]` on their own lines.

MiniMax Music 3 has no `bpm`, `keyscale`, `timesignature`, `language`, `shift`,
`composerMode`, or `creativity` controls. Local `generate_music` strips its
ACE-Step-only fields and keeps lyrics, duration, format, seed, and prompt
strength. A model-less request above 300 seconds selects ACE-Step XL Turbo;
an explicitly named Music 3 request remains explicit for service validation.
Speech models are never music candidates. Use an explicit ACE-Step model for
its controls or drafts; ACE-Step accepts 10–600 seconds (default 30).

The `workflow_text_to_music` example defaults to Music 3. Its optional tempo,
key, meter, and language flags become Music 3 prompt directions; ACE-only flags
or a duration above 300 seconds select XL Turbo when no model is named. Use
`--dry-run` to inspect the resolved request without submitting a paid project.

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

Ref2VA also has two-stage IDs: `minimax-h3-ref2va-fp8_r2v_2stage` (20 steps)
and `minimax-h3-ref2va-fp8_r2v_balanced_2stage` (8 steps). They use the same
references, sampling and LoRAs as the corresponding one-stage R2V model, at
24 FPS and guidance 1. The output is twice the requested canvas size.

Audio-guide modes require uploaded audio: `ia2v` also requires a first image,
`flfa2v` requires first and last images, and `a2v` takes audio alone. They use
4 steps, guidance 1, 24 FPS, and frames `124 + n*17` in 124–362. Output always
carries the uploaded audio; `audioStart` selects its offset. `audioDuration`
and `generateAudio:false` are rejected. `loras` and matching `loraStrengths`
are supported, preserving caller order. Use
`get_minimax_h3_frames_for_audio_duration(seconds)` for a covering frame count.

MiniMax H3 image-to-video, first/last-frame, audio-guide, and reference-to-video
models accept up to `MINIMAX_H3_MAX_KEYFRAMES` (8) intermediate stills. Check
`is_minimax_h3_keyframe_model(model_id)` for the supported models.

```rust,no_run
use sogni_client::{AssetRole, MediaSource, MinimaxH3Keyframe, ProjectRequest};

fn pinned_video() -> ProjectRequest {
    ProjectRequest::video(
        "minimax-h3-ref2va-fp8_r2v",
        "Use <Picture 1> for the subject. At 2 seconds, cut to the close-up \
         in <Picture 2>; at 4 seconds, cut to the wide shot in <Picture 3>.",
    )
        .duration(6.0)
        .asset(AssetRole::ReferenceImage, MediaSource::Path("subject.png".into()))
        .keyframes(vec![
            MinimaxH3Keyframe::new(MediaSource::Path("close-up.png".into()), 48),
            MinimaxH3Keyframe::new(MediaSource::Path("wide.png".into()), 96),
        ])
}
```

`frame_index` is a unique zero-based frame at 24 FPS, from 1 through the resolved
frame count minus 2. Keep first/last anchors in their workflow's reference-image
roles. `duration` overrides `frames` and snaps to the H3 grid: 6 seconds produces
141 frames. The builder preserves caller order, even when indices are unsorted;
it registers separate `keyframeImage1..8` uploads in that order without replacing
reference slots. Use `<Picture N>` in chronological order after the workflow's
own pictures and describe each still at its time. Changes of framing or lighting
should start a new shot. Keyframes do not count as reference images. Invalid
models, indices, duplicate frames, or oversized lists fail before any upload.
`estimate_video_cost` accepts `keyframeCount` or derives it from a `keyframes`
array; an explicit count takes precedence, including zero. Pricing remains
service-owned.

Seedance 2.5 defaults to 1080p and accepts 480p/720p/1080p, without 4K.
It accepts `.param("outputFormat", "mov")` and
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

Direct tool media accepts HTTPS links from the existing Sogni/S3/CloudFront
hosts, S3 acceleration/dualstack hosts, and four exact Sogni production/staging
R2 input/output buckets. Other R2 customer buckets are refused. Downloads
retain signed queries, omit API credentials, refuse redirects, and enforce
size/format limits; strict media also validates and pins public DNS addresses.

## Personal LoRAs and estimates

`projects.personal_loras()` provides `list`, `get`, `import`, `remove`, and
`catalog`. `ImportPersonalLoraParams` includes the source URL, name, model ID,
and an explicit `rights_confirmed` choice. Import availability, validation and
limits are decided by the service; discover supported targets from `list().models`.

```rust,no_run
use sogni_client::SogniClient;

async fn library(client: &SogniClient) -> sogni_client::Result<()> {
    let library = client.projects.personal_loras().list().await?;
    for lora in library.loras {
        println!("{}: {}", lora.name, lora.status);
    }
    let catalog = client.projects.available_loras_with_personal(None).await?;
    println!("{}", catalog["loras"]);
    Ok(())
}
```

Private catalogs are fetched for each call and never added to a public cache.
`available_loras` remains public-only; `get_lora("personal-...")` reads the
private catalog. Reads and imports reject responses from a previous account.

All estimate helpers accept `billingMode` (`auto` or `tokens`). Video and audio
estimates also accept `network`, defaulting to the current connection's network.
`CostEstimate::daily_fair_use_pct` preserves the service's optional percentage,
including zero. An absent value is not a zero-cost or plan-coverage guarantee.

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
`client.chat.tools.all()` returns the 30 version-pinned hosted definitions,
including `generate_speech`, `upscale_video`, `image_to_3d`, `remove_background`,
and `segment_image`. Hosted chat and durable runs
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

For a durable chat run awaiting cost approval, pass the displayed
`acceptedCostPreview` unchanged to `chat.confirm_run_cost`, together with
`toolCallId` and `decision: "confirm"`. The method never fetches or approves a
replacement preview. Cancellation needs no preview. Set `idempotencyKey` to
reuse the same confirmation operation safely.

For workflow reseeding, set `WorkflowBillingOptions::idempotency_key` and reuse
it only when retrying the same take. `ReseedWorkflowResult::idempotent` is
`Some(true)` when the service reports a replay of that operation.

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

`ApiError::retry_after()` and `ChatError::retry_after()` expose validated server
waits in seconds, preserving fractional body values and falling back to the
`Retry-After` header. `retry_after_seconds` rounds up to whole seconds, and
`details()` returns structured service context when supplied. These accessors
do not retry requests; preserve an operation's idempotency key when retrying.

The SDK does not log credentials or include them in `Debug` output. Server-side
authorization, billing, eligibility, and safety decisions remain authoritative.

External generation errors preserve an optional `vendorFailureCategory` string
in job/project error payloads and public error events. Use it for coarse
messages such as content-policy, input-validation, or timeout failures; handle
unknown categories with a generic error message. Private provider failure
details and response bodies are excluded from these public socket events.

`projects.resolve_missing_with_advice(&ids, None).await` adds per-project
`ProjectRecoveryAdvice` with HTTP `status` and optional whole-second
`retry_after_seconds` for inconclusive lookups. Its optional `ResolveMissingOptions`
uses `attempts` and `retry_delay` (`std::time::Duration`). `resolve_missing` retains its
existing result shape. Transport errors or an unavailable live registry do not
prove a project was lost; recovery keeps waiting without cancelling it.

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
