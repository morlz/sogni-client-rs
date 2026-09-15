# Contributing

Open issues and pull requests in
[Sogni-AI/sogni-client-rs](https://github.com/Sogni-AI/sogni-client-rs).
This is an alpha fork of @morlz's community client. See [UPSTREAM.md](UPSTREAM.md)
for attribution and the upstream update process.

Use a feature branch and keep changes focused. Never include credentials,
private media URLs, or account data in code, logs, or fixtures. Loopback tests
use synthetic credentials. Live generation requires your own Sogni account
and may consume credits; it is not run by public CI.

## Checks

Rust 1.88 or newer and Python 3.11 or newer are required for development.

```bash
cargo fmt --all --check
cargo test --all-targets --all-features --locked
cargo test --all-targets --no-default-features --locked
cargo test --doc --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --no-default-features --locked -- -D warnings
cargo doc --lib --examples --all-features --no-deps --locked
cargo audit --deny warnings
python3 .github/scripts/test_release.py
cargo package --locked
```

## Alpha releases

Update Cargo.toml, Cargo.lock, CHANGELOG.md, ALPHA_RELEASE.md, and the README's
pinned installation tag in a PR. Merge after review and passing CI. From the
primary clean checkout of `main`, run the **GitHub alpha release** workflow.
It verifies the version, tests and audits the package, preserves an immutable
tag, and publishes a GitHub prerelease. It cannot publish to crates.io.
Each release must state what was tested and the remaining validation limits.
