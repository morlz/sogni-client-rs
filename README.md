# Sogni Client for Rust

An asynchronous Rust SDK for the Sogni Supernet and Sogni Intelligence APIs.
It tracks the public wire contract of `sogni-client` and
`sogni-client-python` at version **5.27.1**, while exposing Rust-native typed
errors, streams, snapshots, and builders.

## What is included

- API-key, JWT/refresh-token, cookie, and username/password authentication
- Reconnecting authenticated WebSocket transport with exponential backoff
- Image, video, and audio project submission, progress, results, cancellation,
  uploads, cost estimates, model metadata, LoRAs, and reconnect recovery
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

- Rust 1.85 or newer
- Tokio runtime
- A Sogni account token pair or API key

The default `wallet` feature enables deterministic Sogni wallet derivation and
EIP-712 signing for username/password login, account creation, deposits, and
withdrawals. Disable default features if an application only uses API keys or
pre-issued tokens:

```toml
[dependencies]
sogni-client = { version = "5.27.1", default-features = false }
```

Until the crate is published, use the repository directly:

```toml
[dependencies]
sogni-client = { git = "https://github.com/Sogni-AI/sogni-client-rs", tag = "v5.27.1" }
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
does not retry generation requests.

The service requires the `sogni-client` WebSocket protocol-family identifier for
API-key authentication. The wire field uses that family with the actual compatible
version; HTTP User-Agent remains `sogni-client-rs/<version>`. A WebSocket upgrade
alone is not authentication: `is_socket_authenticated()` becomes true only after
the server's authenticated event. `abort()` synchronously stops the shared
client's transport and pending sends; it does not cancel remote projects. Scoped
execution owners should abort on lease loss and reconcile persisted project IDs.

Token authentication is available through
`SogniClient::builder().tokens(token, refresh_token)`. For an interactive
username/password flow, create a token-auth client and call
`client.account.login(username, password)`.

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

Completed image jobs also support `job.enhance("light", overrides).await`;
the returned enhancement is tracked through `job.enhancement_project()`.

## Upload an input asset

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest};

# fn request() -> ProjectRequest {
ProjectRequest::image("qwen_image_edit_2511_fp8_lightning", "Make it cinematic")
    .asset(
        AssetRole::ContextImage(1),
        MediaSource::Path("input.png".into()),
    )
# }
```

Adding an asset automatically enables its matching request flag. MiniMax H3
reference-to-video supports explicit numbered audio/video slots:

```rust,no_run
use sogni_client::{AssetRole, MediaSource, ProjectRequest};

# fn request() -> ProjectRequest {
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
# }
```

Numbered H3 audio/video slots must be contiguous from slot 1. The SDK validates
H3's frame lattice, dimensions, fixed FPS/steps/guidance, reference limits, and
duration hints before uploading. It also applies the current Wan 3, Seedance,
and HappyHorse task, asset, and duration constraints locally.

Generation assets use the legacy presigned `PUT` flow required by the Supernet
socket protocol. For independent uploads used by durable chat or workflows,
request a current v2 multipart form with `image_upload_post` or
`media_upload_post`, then call `upload_presigned`.

## Stream a chat completion

```rust,no_run
use futures_util::StreamExt;
use serde_json::json;
use sogni_client::SogniClient;

# async fn run(client: &SogniClient) -> sogni_client::Result<()> {
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
# Ok(()) }
```

Use `create_completion` for a non-streaming socket call and
`create_hosted_completion` for the hosted OpenAI-compatible REST endpoint.
`HostedTools::all()` returns the exact version-pinned tool definitions bundled
with this crate.

## Durable chat and workflows

Durable operations accept only retrievable HTTP(S) media references. The client
rejects inline `data:` media before submission because it cannot survive a
disconnect and resume.

```rust,no_run
use futures_util::StreamExt;
use serde_json::json;

# async fn run(client: &sogni_client::SogniClient) -> sogni_client::Result<()> {
let run = client.chat.create_run(&json!({
    "messages": [{"role": "user", "content": "Create a launch campaign"}],
    "confirmCost": false
})).await?;

let run_id = run["id"].as_str().expect("server run id");
let mut events = client.chat.stream_run_events(run_id, None).await?;
while let Some(event) = events.next().await {
    println!("{:#?}", event?);
}
# Ok(()) }
```

The workflow namespace provides `start`, `get`, `list`, `events`,
`stream_events`, `confirm_cost`, `resume`, `reseed`, and `cancel`; saved recipes
are available under `client.workflows.templates`.

## Events and state

`SogniClient`, `ProjectsApi`, `Project`, `Job`, `ChatApi`, and `CurrentAccount`
offer broadcast receivers through `subscribe()`. Lagged receivers are explicit:
handle `tokio::sync::broadcast::error::RecvError::Lagged` and refresh a snapshot
when necessary. Project tracking automatically requests a durable recovery
snapshot after socket authentication and after an internal receiver lag.

Project and job handles are cheap clones backed by synchronized state. Use
`snapshot()` when an immutable, serializable view is needed.

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

# fn inspect(error: Error) {
if is_subscription_limit_error(&error) {
    // Prompt for the appropriate subscription change.
} else if let Error::Api(api) = &error {
    eprintln!("HTTP {}: {}", api.status, api.message);
}
# }
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

See [Sogni documentation](https://docs.sogni.ai/),
[`sogni-client`](https://github.com/Sogni-AI/sogni-client), and
[`sogni-client-python`](https://github.com/Sogni-AI/sogni-client-python) for the
other supported clients and service-level concepts.

## License

ISC
