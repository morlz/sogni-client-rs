# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows the upstream Sogni client version where its public
wire contract is compatible.

## Unreleased

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
