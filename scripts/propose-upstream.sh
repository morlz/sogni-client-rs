#!/usr/bin/env bash
set -euo pipefail

# Create a review PR with the maintainer's gh login. No local checkout changes,
# upstream code execution, merging, or release publication occurs here.
repo=Sogni-AI/sogni-client-rs
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
existing=$(gh pr list --repo "$repo" --base main --state open --json url,headRefName,headRepositoryOwner --jq '.[] | select(.headRefName == "dev" and .headRepositoryOwner.login == "morlz") | .url' | head -1)
if [[ -n "$existing" ]]; then
  printf '%s\n' "$existing"
  exit 0
fi
ahead=$(gh api "repos/$repo/compare/main...morlz:dev" --jq .ahead_by)
if [[ "$ahead" == "0" ]]; then
  echo "All upstream commits are already included."
  exit 0
fi
gh pr create --repo "$repo" --draft --base main --head morlz:dev --title "Sync community upstream updates" --body-file "$script_dir/../.github/UPSTREAM_PR.md"
