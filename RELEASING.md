# Releasing `sogni-client`

Every push to the default branch of `morlz/sogni-client-rs` runs CI. Once the
Linux, macOS, Windows, Rust 1.85, package, and release-automation checks pass,
CI calls the publish workflow from that same commit. A new crate version is
verified, tagged as `vVERSION`, and published to crates.io automatically.
Pull requests, other branches, forks, and tag pushes cannot start publication.

## GitHub configuration

The repository secret is named `CARGO_REGISTRY_TOKEN`. It must contain a valid
crates.io token authorized to publish `sogni-client`. No GitHub Environment or
manual approval is required by this workflow. The tag job requests
`contents: write`; verification and publication use `contents: read`.

For an existing crate, restrict the registry token to publishing this crate.
The first publication may require a token authorized to create the crate;
afterward replace it with a crate-scoped token. The workflow cannot inspect a
GitHub secret's value or validate its registry permissions before publishing.

The registry token is exposed only to the final `cargo publish --locked
--no-verify` command. Tests, builds, documentation, package verification, and
the publish dry run execute without it. The publish command skips compilation
because the same immutable source already passed package verification.

## Prepare a release

1. Complete the upstream contract port and relevant Rust changes.
2. Update `package.version` in `Cargo.toml` and the root package version in
   `Cargo.lock` together. Use the synchronized upstream version when available.
   For Rust-only fixes after that version is published, increment the patch
   version to an unused version greater than the current release. If that
   consumes a later upstream version, use the next available greater version
   when that upstream port is complete. Track the actual upstream version
   independently in `.github/upstream-sync.json`.
3. Add `## [VERSION] - YYYY-MM-DD` to `CHANGELOG.md`. A version already present
   on crates.io is skipped; publishing changed code requires a new version.
4. After complete parity, update `.github/upstream-sync.json` with the exact
   upstream repository, version, commit, and synchronization date. The weekly
   Codex task compares against this baseline before porting later changes.
5. Commit the source, manifest, lockfile, changelog, and synchronization baseline
   together locally so Cargo can package a clean working tree. Keep the commit
   local until the following validation succeeds.
6. Run the local gates with `RUSTDOCFLAGS=-Dwarnings`:

   ```console
   python .github/scripts/test_release.py
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

7. Inspect the `.crate` archive under `target/package`; it must contain only
   the intended public SDK, documentation, examples, manifest, and test media.
8. Push the validated commit to the default branch. Watch the CI run through its
   release jobs, then verify the new version on crates.io and docs.rs.

## Release identities and retries

The workflow checks the exact CI commit, its membership in the current default
branch, the package/repository identity, matching manifest and lockfile versions,
and the changelog heading. It queries the exact version on crates.io before
running publication-specific checks. A missing version proceeds; a published
version skips successfully; an unexpected response fails the run.

The tag job creates `vVERSION` at the verified commit. Existing lightweight or
annotated tags are accepted only if they resolve to that same commit. A tag
pointing anywhere else fails the release; tags are never moved or overwritten.
The publication job checks out the verified commit by SHA and uses the same
locked package. A single concurrency group serializes releases and never
cancels a running publication.

To retry a failed release, rerun its CI workflow. If a failure occurs after tag
creation, rerun the same commit: the existing matching tag is safe to reuse.
If code changes are required, preserve that tag and choose a new version.
If Cargo times out after upload, check the registry first; the upload may have
succeeded even if index polling did not finish. An already-published version
will be skipped on a full rerun. Never overwrite a registry version or reuse its
tag for changed code.

The workflow uses GitHub's [reusable workflow contract](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows)
and Cargo's [publish command](https://doc.rust-lang.org/cargo/commands/cargo-publish.html).
