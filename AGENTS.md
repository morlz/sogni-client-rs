# Repository guidance

This is the Sogni-AI alpha fork of @morlz’s community Rust client. Preserve
upstream history, ISC licensing, and the attribution in README.md and UPSTREAM.md.

- Rust 2024, minimum Rust 1.88, Tokio async runtime. Start at src/lib.rs.
- Core modules: client, auth, transport, account, projects, chat, workflows, replay.
- Follow existing Rust conventions; format with cargo fmt. Unsafe code is forbidden.
- Use feature branches and PRs; never push directly to main or merge without maintainer approval.
- Run the checks in CONTRIBUTING.md. Cover both wallet and no-default-features builds.
- Use synthetic loopback fixtures for auth and transport tests. Never commit credentials or private data.
- Keep server-owned authorization and billing decisions on the server.
- Preserve wire compatibility; verify public source and package contents before releasing.
- Publish only from the primary clean main checkout after the release PR is merged.
- This fork uses GitHub alpha releases; Cargo registry publication stays disabled.
- Review upstream updates as PRs. Never auto-merge or force-reset to upstream.
