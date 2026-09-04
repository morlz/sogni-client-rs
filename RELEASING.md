# Releasing `sogni-client`

The crates.io release is driven only by an immutable `v*` Git tag. The
workflow verifies the source and package without credentials, then makes the
single publish call from the protected `crates-io` GitHub Environment.

## Current version caveat

Do not publish the current `5.27.1` manifest as-is. This Rust tree includes
public contract work added after the TypeScript `v5.27.1` tag, including the
announcements surface. Coordinate the next upstream-compatible version first;
then update `Cargo.toml`, regenerate `Cargo.lock`, add the matching changelog
entry, and tag that reviewed commit. This note does not claim that version
`5.27.1` has been published to crates.io. The publish workflow deliberately
rejects that version so an existing tag cannot bypass this release decision.

Before configuring the release Environment or pushing a tag, also confirm that the
workflow is running from the final official repository and its intended
default branch. The package metadata currently names
`Sogni-AI/sogni-client-rs`; crates.io and the GitHub Environment must be
configured for the repository that actually owns the release workflow.

## One-time GitHub setup

Create a GitHub Environment named `crates-io` with:

- required reviewers and prevention of self-review;
- deployment branches/tags restricted to `v*` tags;
- administrator bypass disabled where organization policy permits it; and
- an Environment secret named `CRATES_IO_TOKEN`.

Use a crates.io API token with the smallest available scope. For an existing
crate, scope it to `sogni-client` and publishing only. crates.io cannot scope a
token to a crate that does not exist yet, so the first publication requires a
short-lived owner token with only the minimum bootstrap capability. Delete or
rotate that bootstrap token immediately after the first successful release and
replace it with a crate-scoped token in the Environment.

The workflow deliberately exposes `CRATES_IO_TOKEN` only to the final
`cargo publish --locked` step. The verify job has read-only repository
permissions, no release Environment, and no registry secret.
The final step uses `--no-verify` because archive verification already passed
without secrets; this prevents compilation or build scripts from running with
the registry token in their environment.

## Prepare a release

1. Confirm the Rust public contract matches the coordinated TypeScript and
   Python release contract.
2. Choose that release version and update `package.version` in `Cargo.toml`.
3. Run `cargo +1.85.0 check` so the root package entry in `Cargo.lock` is
   updated to exactly the same version.
4. Add `## [VERSION] - YYYY-MM-DD` to `CHANGELOG.md` with the public changes.
5. Set `RUSTDOCFLAGS=-Dwarnings` for the documentation commands, then run the
   local gates:

   ```console
   cargo +1.85.0 fmt --all --check
   cargo +1.85.0 test --all-targets --all-features --locked
   cargo +1.85.0 test --all-targets --no-default-features --locked
   cargo +1.85.0 clippy --all-targets --all-features --locked -- -D warnings
   cargo +1.85.0 clippy --all-targets --no-default-features --locked -- -D warnings
   cargo +1.85.0 build --examples --all-features --locked
   cargo +1.85.0 build --examples --no-default-features --locked
   cargo +1.85.0 doc --lib --examples --all-features --no-deps --locked
   cargo +1.85.0 doc --lib --examples --no-default-features --no-deps --locked
   cargo +1.85.0 package --package sogni-client --locked
   cargo +1.85.0 publish --package sogni-client --registry crates-io --locked --dry-run
   ```

6. Inspect the `.crate` archive under `target/package`; it must contain only
   the intended public SDK, documentation, examples, manifest, and test media.
7. Commit the version, lockfile, and changelog together, merge that commit to
   the repository's default branch, and wait for CI to pass.

## Tag and publish

From the reviewed default-branch commit, create and push the matching tag:

```console
git tag -s vVERSION -m "sogni-client vVERSION"
git push origin vVERSION
```

The publish workflow rejects a tag unless all of these identities match:

- tag name `vVERSION`;
- `Cargo.toml` package version;
- the root `sogni-client` version in `Cargo.lock`;
- a `## [VERSION]` changelog heading; and
- a tag commit contained in the repository's current default branch.

It then runs Rust 1.85 formatting, both feature-mode test and lint matrices,
example builds, warning-fatal documentation, package verification, and a
crates.io publish dry run. Only after those gates pass can an Environment
reviewer authorize the publish job.

## Reruns and failures

The workflow queries the exact crate version before entering the protected
publish job. An already-published version is reported and skipped successfully;
a missing version proceeds; any unexpected registry response fails closed.
Per-tag concurrency never cancels an in-progress publication.

crates.io versions are immutable. Never move or reuse a release tag, and never
try to overwrite an existing version. If `cargo publish` times out after upload,
check the crates.io version page before rerunning—the registry may have accepted
the archive even though index polling did not finish.

After publication, verify the version and rendered documentation on crates.io
and docs.rs, then create the normal GitHub release notes from the same tag.
