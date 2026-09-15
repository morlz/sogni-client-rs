# Changelog

## [5.50.3-alpha.1]

- Match the TypeScript client's `0x`-prefixed wallet signature format.
- Start the attributed Sogni-AI community alpha fork from morlz/sogni-client-rs 5.50.2.
- Keep account sessions consistent across token refresh, REST responses, and WebSocket reconnects.
- Apply matching cookies to WebSocket upgrades and idle timeouts to SSE streams.
- Update dependencies, require Rust 1.88, and add regression and upstream-intake checks.
- Distribute this alpha as a GitHub prerelease; preserve the upstream crates.io package.

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows the upstream Sogni client version where its public
wire contract is compatible.

## Unreleased

## [5.50.2] - 2026-09-15

- Express the weekly synchronization time in UTC in the README.

## [5.50.1] - 2026-09-15

- Remove visible rustdoc `#` setup markers from README examples and indent their Rust code for GitHub and crates.io rendering, preserving documentation test coverage.

## [5.50.0] - 2026-09-15

- Port the public TypeScript 5.50.0 contract through upstream `452e789`.
- Publish new crate versions automatically after the default branch passes CI, using immutable version tags.
- Record the exact upstream synchronization baseline for the weekly update task.
- Extend SAM3 with cutouts, bounded instance selection, labeled boxes, and per-selection provenance; accept and omit explicit multimask false outside point prompts.
- Add deterministic BiRefNet background removal and reject enhancement of masks, cutouts, and model artifacts before downloading results.
- Add Pixal3D geometry/texture options, the current single-view graph selector, and multi-view orbit uploads with fixed named slots; suppress all artifact previews.
- Add Qwen3-TTS speech and voice-clone request controls, audio uploads, and model-option mapping without invented music controls.
- Add FlashVSR promptless video upscaling with optional verified source timing, fractional FPS, no client clip-length cap, and detail/speed/seed controls.
- Add GPT Image 2.5 Sunburst/Flare quality, transparency, compression, ordered references, and uploaded/data-URI edit masks.
- Support the final FastH3 two-stage and audio-guide model IDs, exact upload requirements, and an audio-duration frame helper; reject retired outputScale and 720p request IDs.
- Add Seedance 2.5 MOV output and separately exported final frames, including signed URL refresh and persisted output-format metadata.
- Reuse verified private subscriber uploads, expose the saved-upload API, and keep transfers and availability scoped to the current account.
- Preserve logical Job handles across worker reassignment and ignore superseded attempt events.
- Recover confirmed chat streams across socket restarts, surface retryable lost-turn errors, and reconcile explicit pre-admission project restart refusals.
- Sync 27 hosted tools and model routing from Sogni Protocol alpha.42, including speech, video upscaling, GPT Image 2.5, and FastH3 audio-guide/two-stage selectors.
- Expose model-selection helpers and preserve explicit workflow content-filter preferences.
- Keep generation-receipt validation limited to public wire shape and hashes; service authorization and model eligibility remain authoritative.
- Surface plain-text REST error explanations and omit unset query parameters.
- Add reproducible upstream serializer, model-tier, hosted-tool, and model-routing parity fixtures.
- Add SAM3 prompts and preflight validation with a single lossless PNG mask per source.
- Deliver Pixal3D GLB artifacts through the media API, including recovered results.
- Expose worker result provenance, Sogni World receipt requests, and typed model preparation phases.
- Allow REST-only startup without an app ID; require an explicit stable ID for WebSocket clients.
- Reuse stable example identities, with `SOGNI_APP_ID` available for separate installations.
- Preserve terminal recovery reasons and settle unfinished children and completion waiters.
- Reject retired Wan 3 `smartDuration` whenever present, including false and null.
- Keep omitted diffusion settings under model defaults and preserve null Comfy sampler/scheduler fields.
- Preserve the advertised boolean upscale capability from model tiers in normalized model options.
- Add application-reserved UUID submission and exact project recovery for durable callers.
- Keep missing-project recovery inconclusive when the live registry is unavailable or malformed.
- Expose socket connectivity so applications can avoid billing unreliable disconnected intervals.
- Redact upstream error bodies from background transport, account and recovery logs.
- Allow HTTP-only catalogue clients to defer realtime connection until a socket command.
- Select the WebSocket TLS crypto provider explicitly when host dependencies enable both backends.
- Use the service's required WebSocket protocol family while retaining the Rust HTTP User-Agent.
- Expose server-confirmed socket authentication and synchronous transport abort for scoped owners.
- Route REST, media, and WebSocket traffic through an explicit SOCKS5/SOCKS5h proxy.
- Honor configured timeouts on media transfers and add opt-in strict public-HTTPS media pinning.
- Preserve the canonical image-worker null/array defaults and image-upload MIME contract.
- Leave absent workload attribution unset while preserving explicit caller metadata.
- Allow callers to reserve guide-image UUIDs and verify an uploaded input before submission.
- Add redacted phase-aware submission failures while preserving the existing create error contract.
- Retry idempotent asset PUTs with identical bytes under one bounded request deadline.
- Reconcile owner-scoped v2 active and compact terminal project status without treating HTTP success as completion.
- Confirm cancellation through terminal owner status rather than its acknowledgment alone.
- Recover missing image results into existing completed project handles without resubmission.
- Preserve terminal project and image states across late progress events and expired result lookups.

## [5.27.1] - 2026-09-04

### Added

- Initial production-ready Rust port of the TypeScript and Python Sogni clients.
- API-key, bearer-token, refresh-token, and cookie-compatible authentication.
- REST, WebSocket, and SSE transports with reconnection and structured errors.
- Generation projects/jobs, hosted and socket chat, durable runs/workflows,
  templates, replay, account, statistics, announcement, and upload APIs.
- Wire-compatible video preflight for MiniMax H3, Wan 3, Seedance, and
  HappyHorse, including numbered H3 reference-media uploads.
- Cross-platform CI, minimum-supported-Rust checks, package auditing, and
  credential-redaction regression coverage.
- Version-aware subscription projections that prevent stale REST responses
  from overwriting newer socket entitlement events.
