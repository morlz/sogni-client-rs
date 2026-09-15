"""GitHub alpha release identity and immutable tag checks."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib
import urllib.error
import urllib.request

REPOSITORY = "Sogni-AI/sogni-client-rs"
PACKAGE = "sogni-client"
VERSION_PATTERN = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?"


def require_release_dispatch(environment):
    if environment.get("GITHUB_REPOSITORY") != REPOSITORY:
        raise ValueError("Publishing is restricted to the release repository.")
    branch = environment.get("DEFAULT_BRANCH")
    if branch != "main" or environment.get("GITHUB_EVENT_NAME") != "workflow_dispatch":
        raise ValueError("Publishing requires manual dispatch from main.")
    if environment.get("GITHUB_REF") != f"refs/heads/{branch}":
        raise ValueError("Publishing requires manual dispatch from main.")
    if not re.fullmatch(r"[0-9a-f]{40}", environment.get("GITHUB_SHA", "")):
        raise ValueError("Publishing requires an exact source commit.")


def require_version(version):
    if not isinstance(version, str) or not re.fullmatch(VERSION_PATTERN, version):
        raise ValueError("The release version must be a semantic version without build metadata.")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+-alpha\.[1-9][0-9]*", version):
        raise ValueError("This fork publishes alpha versions only.")
    return version


def release_version(root):
    package = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    if package["name"] != PACKAGE:
        raise ValueError("Unexpected package name.")
    if package["repository"] != f"https://github.com/{REPOSITORY}":
        raise ValueError("Cargo.toml must identify the release repository.")
    if package.get("publish") is not False:
        raise ValueError("Cargo registry publication must remain disabled.")
    version = require_version(package["version"])
    locked = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))["package"]
    if not any(p["name"] == PACKAGE and p["version"] == version and "source" not in p for p in locked):
        raise ValueError("Cargo.lock must match the package release version.")
    changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    if not re.search(rf"^## \[{re.escape(version)}\](?: - \d{{4}}-\d{{2}}-\d{{2}})?\s*$", changelog, re.M):
        raise ValueError("CHANGELOG.md must contain the release heading.")
    return version


def request_json(url, *, token=None, body=None, missing_ok=False):
    headers = {"User-Agent": "sogni-client-rs-release", "Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = None if body is None else json.dumps(body).encode("utf-8")
    if data is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, headers=headers, data=data)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if missing_ok and error.code == 404:
            return None
        raise ValueError(f"Release API returned HTTP {error.code}.") from None
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        raise ValueError(f"Release API request failed ({type(error).__name__}).") from None


def already_published(version, request=request_json):
    require_version(version)
    record = request(f"https://api.github.com/repos/{REPOSITORY}/releases/tags/v{version}", missing_ok=True)
    if record is None:
        return False
    if not isinstance(record, dict) or record.get("tag_name") != f"v{version}" or not record.get("prerelease"):
        raise ValueError("GitHub did not return the expected alpha prerelease.")
    if record.get("draft"):
        raise ValueError("A draft release exists; review it before publishing.")
    return True


def ensure_tag(version, commit, token, request=request_json):
    require_version(version)
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("The release tag requires an exact source commit.")
    if not token:
        raise ValueError("The GitHub token is missing.")
    api = f"https://api.github.com/repos/{REPOSITORY}/git"
    ref_url = f"{api}/ref/tags/v{version}"
    tag = request(ref_url, token=token, missing_ok=True)
    if tag is None:
        request(f"{api}/refs", token=token, body={"ref": f"refs/tags/v{version}", "sha": commit})
        tag = request(ref_url, token=token)
    target = tag["object"]
    # Existing signed/annotated tags are allowed only when they resolve to this
    # same verified commit. A release never moves or replaces a tag.
    for _ in range(16):
        if target["type"] == "commit":
            if target["sha"] != commit:
                raise ValueError("The version tag already belongs to a different commit.")
            return
        if target["type"] != "tag":
            break
        target = request(f"{api}/tags/{target['sha']}", token=token)["object"]
    raise ValueError("The version tag does not resolve to the verified commit.")


def output(name, value):
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as destination:
        destination.write(f"{name}={value}\n")


def verify_source():
    require_release_dispatch(os.environ)
    branch = os.environ["DEFAULT_BRANCH"]
    commit = os.environ["GITHUB_SHA"]
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != commit:
        raise ValueError("The checked-out source does not match the CI commit.")
    subprocess.run(["git", "fetch", "--no-tags", "origin", f"refs/heads/{branch}:refs/remotes/origin/{branch}"], check=True)
    subprocess.run(["git", "merge-base", "--is-ancestor", commit, f"origin/{branch}"], check=True)
    version = release_version(Path.cwd())
    published = already_published(version)
    output("version", version)
    output("already_published", str(published).lower())
    if published:
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a", encoding="utf-8") as summary:
            summary.write(f"{PACKAGE} {version} is already published; publication is skipped.\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["verify", "tag", "published"])
    args = parser.parse_args()
    if args.command == "verify":
        verify_source()
    elif args.command == "tag":
        require_release_dispatch(os.environ)
        ensure_tag(os.environ["VERSION"], os.environ["GITHUB_SHA"], os.environ.get("GITHUB_TOKEN"))
    elif not already_published(os.environ["VERSION"]):
        raise ValueError("The released version is not available on GitHub yet.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from None
