# Community origin and upstream updates

## Attribution

This is Sogni-AI's alpha fork of
[morlz/sogni-client-rs](https://github.com/morlz/sogni-client-rs), originally
created by [@morlz](https://github.com/morlz) with AI assistance. Thank you for
the implementation and for making it available to the Sogni community.

Initial baseline: upstream `dev`, version `5.50.2`, commit
`76f5b6793bbca3c7bb70f1ad4d455379ddb50a7b`. Git history and the original ISC
license are preserved. Sogni-AI maintains this fork; this does not transfer
ownership of the upstream repository or its crates.io package.

## Intake

The **Upstream updates** workflow checks weekly and on manual dispatch. It
opens or updates an issue when the author's `dev` has commits absent from
`main`, with a compare link and a command to create a draft intake PR:

```bash
bash scripts/propose-upstream.sh
```

Sogni-AI organization policy blocks PR creation by GitHub Actions, so the
command uses the maintainer's existing GitHub CLI login. It opens a PR from
`morlz:dev` into this fork's `main` without changing the local checkout.
Normal PR CI runs under the maintainer-triggered event. The PR follows the
upstream branch; check its final revision before approval.

Review the diff, resolve conflicts, pass CI, and approve before merging. Use
a merge commit to preserve upstream ancestry; do not squash upstream intake
PRs. Never accept an upstream history rewrite without investigation. Close
the update notice after the PR is merged or the update is explicitly deferred.

The workflow never runs upstream code, merges, or releases. It only needs
repository read access and issue write access; Actions must be enabled on the
fork. Release permissions are scoped separately.

## Local fallback

Configure `origin` as this fork and `upstream` as the author's repository:

```bash
git remote add upstream https://github.com/morlz/sogni-client-rs.git
git fetch upstream dev
git switch -c sync/community-update origin/main
git merge --no-ff upstream/dev
```

If `upstream` already exists, start with `git fetch`. Resolve conflicts while
preserving fork attribution, alpha distribution, session fixes, and tests.
Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md), push the feature branch,
and open a PR into this fork's `main`. Never replace `main` with upstream or
force-push it. Contributions back to upstream should be separate focused PRs.

## Releases

Fork versions are independent of upstream; matching version prefixes do not
promise identical behavior or full parity with other language SDKs. Alpha
releases use immutable `vX.Y.Z-alpha.N` tags and GitHub prereleases. Cargo
registry publication is disabled to avoid confusing this fork with the
author's crates.io package. Registry ownership or naming would be a separate
agreement with the author.
