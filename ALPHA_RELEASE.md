# 5.50.3-alpha.1 — Community Rust SDK alpha

This is Sogni-AI's experimental fork of the community Rust SDK created by
[@morlz](https://github.com/morlz), based on upstream `5.50.2` at
`76f5b6793bbca3c7bb70f1ad4d455379ddb50a7b`. Thank you for creating the client,
sharing it, and offering it to the Sogni community!

This is an alpha for early adopters. It is not a production support or full
API parity commitment. The original repository and crates.io release remain
separate; original history and ISC attribution are preserved.

## Changes in this fork

- Serialize wallet signatures with the required `0x` prefix for login and
  other signed requests, with an ethers.js interoperability fixture.
- Reconnect WebSockets when the account session changes and scope queued
  commands to their originating session.
- Preserve the current account when an earlier token refresh or HTTP response
  finishes late.
- Send matching login cookies on native WebSocket upgrades.
- Keep healthy SSE streams open beyond the normal request deadline while
  retaining connection and idle-read timeouts.
- Update Rustls, time, and anyhow dependencies; require Rust 1.88 or newer.
- Add regression coverage and a reviewed upstream-update process.

## Validation

Local validation on macOS includes 241 passing full-feature tests across the
library, regression suite, and examples; 234 tests pass without wallet support,
and 12 documentation examples compile. Release checks also cover doctests, formatting, Clippy, generated documentation,
package verification, and the RustSec dependency audit. CI is configured for
Linux, macOS, Windows, and the minimum supported Rust version.

Two existing tests requiring explicitly supplied image/upload fixtures remain
ignored in the normal suite.

Production smoke tests passed for username/password login, authenticated
WebSocket operation, image cost estimation, one 512×512 image generation with
progress and result download, and a streaming text-chat completion. Token
switching, delayed refresh responses, cookie upgrades, and SSE timeout behavior
are covered by synthetic loopback tests. API-key and cookie authentication have
not been live-tested in this release.

Wallet transfers, every model and tool, sustained production load, and every
recovery scenario are not certified by this alpha. Start with small workloads
and report reproducible issues in this fork.

## Install

```toml
[dependencies]
sogni-client = { git = "https://github.com/Sogni-AI/sogni-client-rs", tag = "v5.50.3-alpha.1", default-features = false }
```

Remove `default-features = false` for username/password login and wallet
support. Commit your application's Cargo.lock. See [README.md](README.md)
for examples and [UPSTREAM.md](UPSTREAM.md) for provenance and updates.
