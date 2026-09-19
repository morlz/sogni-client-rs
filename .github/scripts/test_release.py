import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import urllib.error

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

SHA = "a" * 40
VERSION = "5.50.0"


class ReleaseTests(unittest.TestCase):
    def test_only_release_repository_default_branch_push_is_trusted(self):
        trusted = {"GITHUB_REPOSITORY": release.REPOSITORY, "DEFAULT_BRANCH": "dev", "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/dev", "GITHUB_SHA": SHA}
        release.require_trusted_push(trusted)
        for key, value in [("GITHUB_REPOSITORY", "fork/sogni-client-rs"), ("GITHUB_EVENT_NAME", "pull_request"), ("GITHUB_EVENT_NAME", "workflow_run"), ("GITHUB_REF", "refs/heads/feature"), ("GITHUB_REF", "refs/tags/v5.50.0"), ("GITHUB_SHA", "dev"), ("DEFAULT_BRANCH", "")]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                release.require_trusted_push({**trusted, key: value})

    def test_manifest_lockfile_changelog_and_repository_must_agree(self):
        root = Path("release-fixture")
        files = {}
        with patch.object(Path, "read_text", lambda path, **kwargs: files[path.name]):
            manifest = f'[package]\nname="{release.PACKAGE}"\nversion="{VERSION}"\nrepository="https://github.com/{release.REPOSITORY}"\n'
            lock = f'[[package]]\nname="{release.PACKAGE}"\nversion="{VERSION}"\n'
            changelog = f'## [{VERSION}] - 2026-09-15\n'
            files = {"Cargo.toml": manifest, "Cargo.lock": lock, "CHANGELOG.md": changelog}
            self.assertEqual(release.release_version(root), VERSION)
            for name in files:
                with self.subTest(name=name):
                    original = files[name]
                    files[name] = original.replace(VERSION, "5.27.1")
                    with self.assertRaises(ValueError):
                        release.release_version(root)
                    files[name] = original
            files["Cargo.toml"] = manifest.replace("morlz/", "other/")
            with self.assertRaises(ValueError):
                release.release_version(root)

    def test_legacy_package_identity_is_rejected_everywhere(self):
        root = Path("release-fixture")
        manifest = f'[package]\nname="{release.PACKAGE}"\nversion="{VERSION}"\nrepository="https://github.com/{release.REPOSITORY}"\n'
        lock = f'[[package]]\nname="{release.PACKAGE}"\nversion="{VERSION}"\n'
        changelog = f'## [{VERSION}] - 2026-09-19\n'
        files = {"Cargo.toml": manifest, "Cargo.lock": lock, "CHANGELOG.md": changelog}
        with patch.object(Path, "read_text", lambda path, **kwargs: files[path.name]):
            for name in ("Cargo.toml", "Cargo.lock"):
                with self.subTest(name=name):
                    original = files[name]
                    files[name] = original.replace(release.PACKAGE, "sogni-client")
                    with self.assertRaises(ValueError):
                        release.release_version(root)
                    files[name] = original

    def test_publish_workflow_targets_only_the_current_package(self):
        workflow = Path(__file__).parents[1] / "workflows" / "publish.yml"
        contents = workflow.read_text(encoding="utf-8")
        self.assertEqual(contents.count(f"--package {release.PACKAGE}"), 4)
        self.assertNotRegex(contents, r"--package sogni-client(?:\s|$)")

    def test_registry_missing_existing_and_mismatched_versions(self):
        self.assertFalse(release.already_published(VERSION, request=lambda *a, **k: None))
        self.assertTrue(release.already_published(VERSION, request=lambda *a, **k: {"version": {"num": VERSION}}))
        for payload in [{}, {"version": {"num": "5.27.1"}}, {"version": None}, []]:
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                release.already_published(VERSION, request=lambda *a, **k: payload)

    def test_registry_http_failures_are_not_missing_versions(self):
        for status in [401, 403, 429, 500, 503]:
            error = urllib.error.HTTPError("https://crates.io", status, "error", {}, None)
            with self.subTest(status=status), patch("urllib.request.urlopen", side_effect=error), self.assertRaises(ValueError):
                release.request_json("https://crates.io", missing_ok=True)
        error = urllib.error.HTTPError("https://crates.io", 404, "missing", {}, None)
        with patch("urllib.request.urlopen", side_effect=error):
            self.assertIsNone(release.request_json("https://crates.io", missing_ok=True))

    def test_existing_tag_cannot_move_to_another_commit(self):
        for target in [{"type": "commit", "sha": "b" * 40}, {"type": "tree", "sha": SHA}]:
            with self.subTest(target=target), self.assertRaises(ValueError):
                release.ensure_tag(VERSION, SHA, "test-token", request=lambda *a, **k: {"object": target})
        release.ensure_tag(VERSION, SHA, "test-token", request=lambda *a, **k: {"object": {"type": "commit", "sha": SHA}})

    def test_annotated_tag_is_peeled_to_its_source_commit(self):
        responses = iter([{"object": {"type": "tag", "sha": "b" * 40}}, {"object": {"type": "commit", "sha": SHA}}])
        release.ensure_tag(VERSION, SHA, "test-token", request=lambda *a, **k: next(responses))

    def test_missing_tag_is_created_then_verified(self):
        calls = []
        responses = iter([None, {}, {"object": {"type": "commit", "sha": SHA}}])
        def request(url, **kwargs):
            calls.append((url, kwargs))
            return next(responses)
        release.ensure_tag(VERSION, SHA, "test-token", request=request)
        self.assertEqual(calls[1][1]["body"], {"ref": f"refs/tags/v{VERSION}", "sha": SHA})
        self.assertNotIn("body", calls[0][1])
        self.assertNotIn("body", calls[2][1])


if __name__ == "__main__":
    unittest.main()
