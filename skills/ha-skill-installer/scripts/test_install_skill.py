"""Offline installation contracts. All writes stay in temporary directories."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import install_skill as installer


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hope-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.data = self.root / "data"
        self.environment = patch.dict(os.environ, {"HA_DATA_DIR": str(self.data)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.source = self.root / "source"
        self.source.mkdir()
        self.skill_text = "---\nname: test-example\ndescription: Test skill\n---\nRead the supplied document.\n"
        (self.source / "SKILL.md").write_text(self.skill_text, encoding="utf-8")
        scripts = self.source / "scripts"
        scripts.mkdir()
        self.sentinel = self.root / "must-not-run"
        (scripts / "example.py").write_text(
            "from pathlib import Path\nPath(%r).touch()\n" % str(self.sentinel), encoding="utf-8")
        (scripts / "example.py").chmod(0o755)

    def prepare(self, **overrides):
        args = dict(local=str(self.source), project=None, repo=None, url=None, path=None, ref=None)
        args.update(overrides)
        result = installer.prepare(argparse.Namespace(**args))
        self.addCleanup(shutil.rmtree, Path(result["plan"]).parent, True)
        return result

    def install(self, plan):
        return installer.install(Path(plan["plan"]), plan["expectedDigest"])

    def test_preparation_is_inactive_and_install_uses_reviewed_snapshot(self):
        plan = self.prepare()
        self.assertFalse(self.data.exists())
        (self.source / "SKILL.md").write_text(self.skill_text + "Unreviewed change")
        result = self.install(plan)
        target = Path(result["target"])
        self.assertEqual(target, self.data / "skills" / "test-example")
        self.assertEqual((target / "SKILL.md").read_text(), self.skill_text)
        self.assertEqual((target / "scripts/example.py").read_bytes(),
                         (self.source / "scripts/example.py").read_bytes())
        if os.name != "nt":
            self.assertTrue((target / "scripts/example.py").stat().st_mode & 0o111)
        receipt = json.loads((target / installer.RECEIPT).read_text())
        self.assertEqual(receipt["previewDigest"], plan["expectedDigest"])
        self.assertEqual(result["next"]["arguments"], {"name": "test-example", "action": "inspect"})
        self.assertEqual(result["previewCleanup"], "removed")
        self.assertFalse(Path(plan["plan"]).parent.exists())
        self.assertFalse(self.sentinel.exists())

    def test_discard_removes_only_the_selected_preview(self):
        plan = self.prepare()
        other = self.prepare()
        result = installer.discard(Path(plan["plan"]), plan["expectedDigest"])
        self.assertEqual(result["status"], "discarded")
        self.assertFalse(Path(plan["plan"]).parent.exists())
        self.assertTrue(Path(other["reviewDirectory"]).exists())
        self.assertEqual((self.source / "SKILL.md").read_text(), self.skill_text)
        self.assertFalse(self.data.exists())
        self.assertFalse(self.sentinel.exists())

    def test_discard_requires_the_original_digest_and_preview_location(self):
        plan = self.prepare()
        with self.assertRaisesRegex(installer.InstallError, "Plan differs"):
            installer.discard(Path(plan["plan"]), "0" * 64)
        copied = self.root / "copied-preview"
        shutil.copytree(Path(plan["plan"]).parent, copied)
        with self.assertRaisesRegex(installer.InstallError, "original installer preview"):
            installer.discard(copied / "plan.json", plan["expectedDigest"])
        self.assertTrue((copied / "plan.json").exists())
        self.assertTrue(Path(plan["reviewDirectory"]).exists())

    def test_discard_preserves_unrelated_review_notes(self):
        plan = self.prepare()
        notes = Path(plan["plan"]).parent / "notes.txt"
        notes.write_text("Keep these notes")
        with self.assertRaisesRegex(installer.InstallError, "unrelated files"):
            installer.discard(Path(plan["plan"]), plan["expectedDigest"])
        self.assertEqual(notes.read_text(), "Keep these notes")
        self.assertTrue(Path(plan["reviewDirectory"]).exists())

    def test_discard_rejects_a_review_directory_replaced_by_a_symlink(self):
        plan = self.prepare()
        review = Path(plan["reviewDirectory"]).parent
        shutil.rmtree(review)
        try:
            review.symlink_to(self.source, target_is_directory=True)
        except OSError:
            self.skipTest("Symlinks unavailable for this test account")
        with self.assertRaisesRegex(installer.InstallError, "Symlink"):
            installer.discard(Path(plan["plan"]), plan["expectedDigest"])
        self.assertEqual((self.source / "SKILL.md").read_text(), self.skill_text)

    def test_install_cleanup_failure_preserves_success_and_can_be_retried(self):
        plan = self.prepare()
        review = Path(plan["reviewDirectory"]).parent
        rmtree = shutil.rmtree

        def remove(path, *args, **kwargs):
            if Path(path) == review:
                rmtree(Path(plan["reviewDirectory"]))
                raise PermissionError("temporary cleanup failure")
            return rmtree(path, *args, **kwargs)

        with patch.object(installer.shutil, "rmtree", side_effect=remove):
            result = self.install(plan)
        self.assertEqual(result["status"], "installed")
        self.assertEqual(result["previewCleanup"], "pending")
        self.assertEqual(result["cleanup"]["expectedDigest"], plan["expectedDigest"])
        target = Path(result["target"])
        self.assertEqual((target / "SKILL.md").read_text(), self.skill_text)
        self.assertTrue(Path(plan["plan"]).exists())
        self.assertFalse(Path(plan["reviewDirectory"]).exists())
        installer.discard(Path(plan["plan"]), plan["expectedDigest"])
        self.assertFalse(review.parent.exists())
        self.assertEqual((target / "SKILL.md").read_text(), self.skill_text)
        self.assertTrue((target / installer.RECEIPT).exists())

    def test_discard_cli_removes_an_abandoned_preview(self):
        plan = self.prepare()
        result = subprocess.run(
            [sys.executable, str(Path(installer.__file__)), "discard", "--plan", plan["plan"],
             "--expected-digest", plan["expectedDigest"]], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["status"], "discarded")
        self.assertFalse(Path(plan["plan"]).parent.exists())
        self.assertTrue(self.source.exists())

    def test_project_scope_is_explicit_and_bound_to_preview(self):
        project = self.root / "project"
        project.mkdir()
        plan = self.prepare(project=str(project))
        with patch.dict(os.environ, {"HA_DATA_DIR": str(self.root / "changed-data")}):
            result = self.install(plan)
        self.assertEqual(Path(result["target"]), project / ".hope-agent/skills/test-example")
        self.assertFalse(self.data.exists())

    def test_plan_tampering_cannot_change_destination(self):
        plan = self.prepare()
        path = Path(plan["plan"])
        changed = json.loads(path.read_text())
        changed["root"] = str(self.root / "unexpected")
        path.write_text(json.dumps(changed))
        with self.assertRaisesRegex(installer.InstallError, "Plan differs"):
            self.install(plan)
        self.assertFalse(self.data.exists())
        self.assertFalse((self.root / "unexpected").exists())

    def test_payload_change_or_addition_invalidates_preview(self):
        for action in ["edit", "add"]:
            with self.subTest(action=action):
                plan = self.prepare()
                payload = Path(plan["reviewDirectory"])
                path = payload / ("scripts/example.py" if action == "edit" else "unexpected.txt")
                path.write_text("Changed after review")
                with self.assertRaisesRegex(installer.InstallError, "differs from the preview"):
                    self.install(plan)
                self.assertFalse(self.data.exists())

    def test_existing_directory_is_preserved_even_if_empty(self):
        plan = self.prepare()
        target = Path(plan["target"])
        target.mkdir(parents=True)
        original_inode = target.stat().st_ino
        with self.assertRaisesRegex(installer.InstallError, "already exists"):
            self.install(plan)
        self.assertEqual(target.stat().st_ino, original_inode)
        self.assertEqual(list(target.iterdir()), [])

    def test_native_publication_does_not_replace_racing_empty_directory(self):
        destination = self.root / "skills"
        destination.mkdir()
        existing = destination / "example"
        existing.mkdir()
        original_inode = existing.stat().st_ino
        candidate = self.root / "candidate"
        candidate.mkdir()
        (candidate / "SKILL.md").write_text(self.skill_text)
        with self.assertRaises((installer.InstallError, FileExistsError)):
            installer.publish_directory(candidate, destination, "example")
        self.assertEqual(existing.stat().st_ino, original_inode)
        self.assertEqual(list(existing.iterdir()), [])
        self.assertTrue(candidate.exists())

    def test_failed_publication_leaves_no_partial_skill(self):
        plan = self.prepare()
        with patch.object(installer, "publish_directory", side_effect=installer.InstallError("simulated failure")):
            with self.assertRaises(installer.InstallError):
                self.install(plan)
        self.assertFalse(Path(plan["target"]).exists())
        self.assertEqual(list(self.data.glob(".hope-skill-install-*")), [])
        self.assertTrue(Path(plan["reviewDirectory"]).exists())

    def test_bundled_name_is_protected(self):
        (self.source / "SKILL.md").write_text(self.skill_text.replace("test-example", "ha-settings"))
        with self.assertRaisesRegex(installer.InstallError, "bundled skill"):
            self.prepare()

    def test_declared_name_conflict_in_differently_named_directory(self):
        existing = self.data / "skills" / "different-folder"
        existing.mkdir(parents=True)
        (existing / "SKILL.md").write_text(self.skill_text)
        with self.assertRaisesRegex(installer.InstallError, "same name"):
            self.prepare()

    def test_case_insensitive_target_collision(self):
        (self.data / "skills" / "TEST-EXAMPLE").mkdir(parents=True)
        with self.assertRaisesRegex(installer.InstallError, "already exists"):
            self.prepare()

    def test_inactive_skill_name_also_prevents_shadowing(self):
        existing = self.data / "skills" / "draft-folder"
        existing.mkdir(parents=True)
        (existing / "SKILL.md").write_text(self.skill_text.replace("description: Test skill", "description: Test skill\nstatus: draft"))
        with self.assertRaisesRegex(installer.InstallError, "same name"):
            self.prepare()

    def test_relative_data_root_does_not_follow_exec_working_directory(self):
        with patch.dict(os.environ, {"HA_DATA_DIR": "relative-data"}):
            with self.assertRaisesRegex(installer.InstallError, "must be absolute"):
                self.prepare()

    def test_reinstall_never_overwrites(self):
        plan = self.prepare()
        self.install(plan)
        target = Path(plan["target"]) / "SKILL.md"
        target.write_text("User edited this")
        with self.assertRaises(installer.InstallError):
            self.install(plan)
        self.assertEqual(target.read_text(), "User edited this")

    def test_source_file_and_directory_symlinks_are_rejected(self):
        for link_target in [self.root, self.source / "SKILL.md"]:
            link = self.source / "link"
            try:
                link.symlink_to(link_target, target_is_directory=link_target.is_dir())
            except OSError:
                self.skipTest("Symlinks unavailable for this test account")
            try:
                with self.assertRaisesRegex(installer.InstallError, "symlink"):
                    self.prepare()
            finally:
                link.unlink()

    def test_unreadable_resource_directory_aborts_preparation(self):
        assets = self.source / "assets"
        assets.mkdir()
        (assets / "required.txt").write_text("required resource")
        scandir = os.scandir

        def read_directory(path):
            if not isinstance(path, int) and Path(path) == assets:
                raise PermissionError("unreadable resource directory")
            return scandir(path)

        with patch.object(installer.os, "scandir", side_effect=read_directory):
            with self.assertRaisesRegex(installer.InstallError, "Cannot read a package directory"):
                self.prepare()
        self.assertFalse(self.data.exists())

    @unittest.skipIf(os.name == "nt", "POSIX directory mode fixture")
    def test_actual_unreadable_directory_is_not_silently_omitted(self):
        if os.geteuid() == 0:
            self.skipTest("Root can read the fixture regardless of its permissions")
        assets = self.source / "assets"
        assets.mkdir()
        (assets / "required.txt").write_text("required resource")
        assets.chmod(0)
        try:
            with self.assertRaisesRegex(installer.InstallError, "Cannot read a package directory"):
                self.prepare()
            self.assertFalse(self.data.exists())
        finally:
            assets.chmod(0o755)

    def test_target_root_symlink_added_after_preview_is_rejected(self):
        plan = self.prepare()
        self.data.mkdir()
        elsewhere = self.root / "elsewhere"
        elsewhere.mkdir()
        try:
            (self.data / "skills").symlink_to(elsewhere, target_is_directory=True)
        except OSError:
            self.skipTest("Symlinks unavailable for this test account")
        with self.assertRaisesRegex(installer.InstallError, "Symlink"):
            self.install(plan)
        self.assertEqual(list(elsewhere.iterdir()), [])

    def test_size_and_file_count_limits(self):
        with patch.object(installer, "MAX_FILES", 1):
            with self.assertRaises(installer.InstallError):
                self.prepare()
        with patch.object(installer, "MAX_FILE_BYTES", 5):
            with self.assertRaises(installer.InstallError):
                self.prepare()
        self.assertFalse(self.data.exists())

    def test_missing_or_invalid_skill_is_not_published(self):
        invalid = [b"No frontmatter", b"---\nname: ../escape\ndescription: x\n---\nBody",
                   b"---\nname: example\nname: other\ndescription: x\n---\nBody",
                   b"---\nname: example\ndescription: []\n---\nBody",
                   b"---\nname: example\ndescription: x\nstatus: draft\n---\nBody"]
        for content in invalid:
            with self.subTest(content=content):
                (self.source / "SKILL.md").write_bytes(content)
                with self.assertRaises(installer.InstallError):
                    self.prepare()
        self.assertFalse(self.data.exists())

    def test_block_description_and_resource_files_are_preserved(self):
        content = b"---\nname: test-example\ndescription: >\n  First line\n  second line\n---\nBody\n"
        (self.source / "SKILL.md").write_bytes(content)
        (self.source / "assets").mkdir()
        (self.source / "assets" / "example.bin").write_bytes(b"\x00\xff\x01")
        plan = self.prepare()
        self.assertEqual(plan["description"], "First line second line")
        result = self.install(plan)
        self.assertEqual((Path(result["target"]) / "SKILL.md").read_bytes(), content)

    def test_vcs_and_cache_files_are_excluded(self):
        for directory in [".git", "__pycache__", "node_modules"]:
            (self.source / directory).mkdir()
            (self.source / directory / "ignored").write_text("cache")
        plan = self.prepare()
        self.assertEqual({f["path"] for f in plan["files"]}, {"SKILL.md", "scripts/example.py"})

    def test_cli_failure_is_structured_and_does_not_echo_source_contents(self):
        result = subprocess.run([sys.executable, str(Path(installer.__file__)), "prepare", "--local", str(self.root)],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stderr)["status"], "error")
        self.assertNotIn("must-not-run", result.stderr)


class SourceParsingTests(unittest.TestCase):
    def test_repository_and_blob_urls(self):
        repo = installer.github_source("owner/repository", None, "skills/example", "v1.2")
        self.assertEqual(repo["ref"], "v1.2")
        blob = installer.github_source(None, "https://github.com/owner/repository/blob/main/skills/example/SKILL.md", None, None)
        self.assertEqual(blob["path"], "skills/example")
        self.assertEqual(installer.github_source("owner/repository", None, None, None)["ref"], "HEAD")

    def test_ref_with_slash_is_separated_from_skill_path(self):
        source = installer.github_source(None, "https://github.com/owner/repository/tree/feature/new-skill/skills/example",
                                         None, "feature/new-skill")
        self.assertEqual(source["ref"], "feature/new-skill")
        self.assertEqual(source["path"], "skills/example")

    def test_urls_cannot_route_to_other_hosts_or_carry_credentials(self):
        for url in ["http://github.com/a/b", "https://token@github.com/a/b", "https://github.com:443/a/b",
                    "https://github.com.evil.example/a/b", "https://github.com/a/b?token=secret",
                    "https://github.com/a/b/tree/main/%2e%2e/private", "https://github.com/a/b/issues/1"]:
            with self.subTest(url=url), self.assertRaises(installer.InstallError):
                installer.github_source(None, url, None, None)

    def test_paths_and_refs_reject_options_and_traversal(self):
        for path in ["../private", "/tmp/file", "C:/file", "a\\b", "a/../b", "a//b", "CON.txt", "a/b."]:
            with self.subTest(path=path), self.assertRaises(installer.InstallError):
                installer.github_source("owner/repo", None, path, "main")
        for ref in ["--upload-pack=evil", "main~1", "main:private", "../main", "a//b"]:
            with self.subTest(ref=ref), self.assertRaises(installer.InstallError):
                installer.github_source("owner/repo", None, "skill", ref)


class GitHubAcquisitionTests(unittest.TestCase):
    commit = "a" * 40
    directory = "b" * 40
    package = "c" * 40
    content = b"---\nname: example\ndescription: test\n---\nBody\n"

    def blob(self, path="SKILL.md", content=None, **changes):
        content = self.content if content is None else content
        oid = hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest()
        return {"path": path, "type": "blob", "mode": "100644", "sha": oid,
                "size": len(content), **changes}

    def acquire(self, entries=None, *, truncated=False, raw_content=None, check=None):
        entries = [self.blob()] if entries is None else entries
        requests = []

        def read(url, limit, deadline):
            requests.append((url, limit))
            if "/commits/" in url:
                return self.commit.encode()
            if "raw.githubusercontent.com" in url:
                return self.content if raw_content is None else raw_content
            oid = self.package
            if "/trees/" + self.commit in url:
                items = [{"path": "skills", "type": "tree", "mode": "040000", "sha": self.directory}]
                oid = self.directory
            elif "/trees/" + self.directory in url:
                items = [{"path": "example", "type": "tree", "mode": "040000", "sha": self.package}]
            else:
                items = entries
            return json.dumps({"sha": oid, "tree": items,
                               "truncated": truncated and "recursive=1" in url}).encode()

        with tempfile.TemporaryDirectory() as directory:
            payload = Path(directory) / "payload"
            try:
                with patch.object(installer, "github_read", side_effect=read):
                    result = installer.acquire_github(
                        {"kind": "github", "repo": "owner/repo", "path": "skills/example", "ref": "feature/new"},
                        payload,
                    )
                self.assertEqual((payload / "SKILL.md").read_bytes(), self.content)
                return result, requests
            finally:
                if check:
                    check(requests, payload)

    def assert_no_content_requested(self, requests, payload):
        self.assertFalse(any("raw.githubusercontent.com" in url for url, _ in requests))
        self.assertFalse(payload.exists())

    def test_metadata_precedes_content_and_downloads_are_pinned_and_capped(self):
        result, requests = self.acquire()
        self.assertEqual(result["commit"], self.commit)
        self.assertTrue(requests[0][0].endswith("/commits/feature%2Fnew"))
        self.assertEqual(requests[-1], (
            "https://raw.githubusercontent.com/owner/repo/" + self.commit + "/skills/example/SKILL.md",
            len(self.content),
        ))
        self.assertEqual(len(requests), 5)

    def test_oversized_file_is_rejected_before_requesting_any_content(self):
        entries = [self.blob(), self.blob("large.bin", size=9 * 1024 * 1024)]
        with self.assertRaisesRegex(installer.InstallError, "exceeds package size limits"):
            self.acquire(entries, check=self.assert_no_content_requested)

    def test_total_size_and_file_count_are_checked_before_download(self):
        entries = [self.blob(), *[self.blob(f"asset-{n}", size=installer.MAX_FILE_BYTES) for n in range(5)]]
        with self.assertRaises(installer.InstallError):
            self.acquire(entries, check=self.assert_no_content_requested)
        entries = [self.blob(), *[self.blob(f"file-{n}", size=0) for n in range(installer.MAX_FILES)]]
        with self.assertRaises(installer.InstallError):
            self.acquire(entries, check=self.assert_no_content_requested)

    def test_missing_skill_and_oversized_frontmatter_are_rejected_before_download(self):
        for entries in [[self.blob("README.md")], [self.blob(size=installer.MAX_SKILL_BYTES + 1)]]:
            with self.subTest(entries=entries), self.assertRaises(installer.InstallError):
                self.acquire(entries, check=self.assert_no_content_requested)

    def test_truncated_tree_is_not_a_complete_snapshot(self):
        with self.assertRaisesRegex(installer.InstallError, "incomplete file tree"):
            self.acquire(truncated=True, check=self.assert_no_content_requested)

    def test_metadata_has_a_cumulative_byte_budget(self):
        with patch.object(installer, "MAX_METADATA_TOTAL_BYTES", len(self.commit)):
            with self.assertRaisesRegex(installer.InstallError, "metadata exceeds"):
                self.acquire(check=self.assert_no_content_requested)

    def test_unsafe_entries_are_rejected_before_content(self):
        for entry in [self.blob(mode="120000"), self.blob("module", type="commit", mode="160000"),
                      self.blob("../escape"), self.blob(path=None), self.blob(size=-1),
                      self.blob(size=True), self.blob(sha="invalid")]:
            with self.subTest(entry=entry), self.assertRaises(installer.InstallError):
                self.acquire([self.blob(), entry], check=self.assert_no_content_requested)
        with self.assertRaises(installer.InstallError):
            self.acquire([self.blob(), self.blob("skill.md")], check=self.assert_no_content_requested)

    def test_cache_files_are_filtered_without_downloading_them(self):
        _, requests = self.acquire([self.blob(), self.blob("node_modules/large.bin", size=100_000_000)])
        self.assertEqual(sum("raw.githubusercontent.com" in url for url, _ in requests), 1)

    def test_content_must_match_both_size_and_git_object(self):
        for content in [self.content + b"extra", self.content.replace(b"Body", b"Evil")]:
            with self.subTest(content=content), self.assertRaisesRegex(installer.InstallError, "differs from the pinned metadata"):
                self.acquire(raw_content=content)


class NativeReaderTests(unittest.TestCase):
    def test_reader_protocol_and_byte_bound_are_verified(self):
        with patch.dict(os.environ, {"HOPE_AGENT_EXECUTABLE": sys.executable}):
            for output in [b"old binary output", installer.FETCH_PREFIX + b"oversized"]:
                response = subprocess.CompletedProcess([], 0, stdout=output, stderr=b"")
                with patch.object(installer.subprocess, "run", return_value=response):
                    with self.assertRaisesRegex(installer.InstallError, "Incompatible or oversized"):
                        installer.github_read("https://api.github.com/repos/a/b/commits/HEAD", 3, installer.time.monotonic() + 10)

    def test_reader_receives_the_per_request_limit_without_credentials(self):
        response = subprocess.CompletedProcess([], 0, stdout=installer.FETCH_PREFIX + b"abc", stderr=b"")
        with patch.dict(os.environ, {"HOPE_AGENT_EXECUTABLE": sys.executable}):
            with patch.object(installer.subprocess, "run", return_value=response) as run:
                content = installer.github_read("https://api.github.com/repos/a/b/commits/HEAD", 3, installer.time.monotonic() + 10)
        self.assertEqual(content, b"abc")
        command = run.call_args.args[0]
        self.assertEqual(command[:2], [sys.executable, "skill-source-fetch"])
        self.assertEqual(command[command.index("--max-bytes") + 1], "3")
        self.assertNotIn("--token", command)

    def test_rate_limit_errors_are_actionable_without_echoing_stderr(self):
        response = subprocess.CompletedProcess([], 1, stdout=b"", stderr=b"skill_source_http_status_403 secret")
        with patch.dict(os.environ, {"HOPE_AGENT_EXECUTABLE": sys.executable}):
            with patch.object(installer.subprocess, "run", return_value=response):
                with self.assertRaisesRegex(installer.InstallError, "rate-limited") as error:
                    installer.github_read("https://api.github.com/repos/a/b/commits/HEAD", 3, installer.time.monotonic() + 10)
        self.assertNotIn("secret", str(error.exception))


if __name__ == "__main__":
    unittest.main()
