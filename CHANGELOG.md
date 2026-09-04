# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows the upstream Sogni client version where its public
wire contract is compatible.

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
