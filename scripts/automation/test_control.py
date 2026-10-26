import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("control", Path(__file__).with_name("control.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ControlTests(unittest.TestCase):
    def symlink(self, link, target, directory=False):
        try:
            link.symlink_to(target, target_is_directory=directory)
        except OSError as error:
            if getattr(error, "winerror", None) == 1314:
                self.skipTest("Windows symlink privilege is unavailable; other controls still run")
            raise

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir()
        (self.runtime / "policy.json").write_text(json.dumps({"task_ids": ["pr", "radar", "code"]}))
        (self.runtime / "tasks.json").write_text("{}")
        self.control = module.Control(self.runtime, self.root / "codex")

    def archive(self, indexed=False):
        root = self.root / "archive"
        for folder in ["reports", "data"]:
            (root / folder / "2026").mkdir(parents=True, exist_ok=True)
        value = {field: [] for field in module.REQUIRED}
        value.update(task_name="fixture", generated_at="2026-10-14T11:00:00+08:00",
                     baseline={"sha": "a" * 40}, no_material_change=True)
        data = json.dumps(value).encode()
        (root / "data/2026/report.json").write_bytes(data)
        (root / "reports/2026/report.md").write_text("# Verified report\n")
        if indexed:
            (root / "LATEST.json").write_text(json.dumps({"generated_at": value["generated_at"],
                "data": "data/2026/report.json", "report": "reports/2026/report.md"}))
            (root / "LATEST.md").write_text("[Report](reports/2026/report.md)\n")
        else:
            (root / "LATEST.json").write_bytes(data)
            (root / "LATEST.md").write_bytes((root / "reports/2026/report.md").read_bytes())
        return root

    def test_index_consumer_resolves_complete_data_and_promotion_preserves_history(self):
        archive = self.archive(indexed=True)
        originals = {str(p): p.read_bytes() for folder in ["data", "reports"] for p in (archive / folder / "2026").iterdir()}
        self.control.registry = {"radar": {"archive_dir": str(archive)}}
        self.control.policy["archive_directories"] = {"radar": str(archive)}
        self.assertEqual(module.archive_state(archive)["status"], "legacy-index")
        result = self.control.normalize_archives()
        self.assertEqual(result["promoted"][0]["status"], "valid")
        self.assertEqual((archive / "LATEST.json").read_bytes(), (archive / "data/2026/report.json").read_bytes())
        for path, content in originals.items():
            self.assertEqual(Path(path).read_bytes(), content)
        self.assertTrue(Path(result["promoted"][0]["backup"]).joinpath("LATEST.json").exists())

    def test_invalid_index_does_not_overwrite_previous_success(self):
        archive = self.archive(indexed=True)
        data = json.loads((archive / "data/2026/report.json").read_text())
        data["generated_at"] = "2026-10-13T11:00:00+08:00"
        (archive / "data/2026/report.json").write_text(json.dumps(data))
        original = (archive / "LATEST.json").read_bytes()
        self.control.registry = {"radar": {"archive_dir": str(archive)}}
        self.control.policy["archive_directories"] = {"radar": str(archive)}
        with self.assertRaisesRegex(ValueError, "generation times differ"):
            self.control.normalize_archives()
        self.assertEqual((archive / "LATEST.json").read_bytes(), original)

    def test_supplementary_payload_is_not_mistaken_for_another_primary_archive(self):
        archive = self.archive()
        folder = archive / "data/2026/report"
        folder.mkdir()
        (folder / "payload.json").write_bytes((archive / "data/2026/report.json").read_bytes())
        self.assertEqual(module.archive_state(archive)["status"], "valid")

    def test_interrupted_two_file_promotion_is_detected_as_a_mixed_pair(self):
        archive = self.archive()
        data = json.loads((archive / "data/2026/report.json").read_text())
        data["generated_at"] = "2026-10-15T11:00:00+08:00"
        encoded = json.dumps(data).encode()
        (archive / "data/2026/new.json").write_bytes(encoded)
        (archive / "LATEST.json").write_bytes(encoded)
        with self.assertRaisesRegex(ValueError, "mixed immutable"):
            module.archive_state(archive)

    def test_immutable_year_must_match_generation_timestamp(self):
        archive = self.archive()
        for folder in ["data", "reports"]:
            (archive / folder / "2026").rename(archive / folder / "2027")
        with self.assertRaisesRegex(ValueError, "year differs"):
            module.archive_state(archive)

    def test_path_traversal_and_symlink_are_rejected(self):
        with self.assertRaises(ValueError):
            module.safe_relative(self.root, "../private.json")
        outside = self.root / "outside"
        outside.mkdir()
        link = self.runtime / "link"
        self.symlink(link, outside, directory=True)
        with self.assertRaises(ValueError):
            module.safe_relative(self.runtime, "link/value.json")

    def test_archive_reader_rejects_latest_and_year_directory_symlinks(self):
        archive = self.archive()
        latest = archive / "LATEST.json"
        outside = self.root / "outside.json"
        outside.write_bytes(latest.read_bytes())
        latest.unlink()
        self.symlink(latest, outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            module.archive_state(archive)
        latest.unlink()
        latest.write_bytes(outside.read_bytes())
        self.symlink(archive / "data/2027", archive / "data/2026", directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            module.archive_state(archive)

    def test_restore_rejects_tampering_and_traversal_before_claiming_success(self):
        source, target = self.root / "source", self.root / "target"
        source.mkdir()
        (source / "config.json").write_bytes(b"original")
        manifest = {"files": [{"path": "config.json", "sha256": module.digest(b"original"), "bytes": 8}]}
        self.assertEqual(module.Control.verify_restore(source, target, manifest), 1)
        (source / "config.json").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            module.Control.verify_restore(source, self.root / "second", manifest)
        manifest["files"][0]["path"] = "../config.json"
        with self.assertRaises(ValueError):
            module.Control.verify_restore(source, self.root / "third", manifest)

    def test_secret_detector_withholds_values(self):
        value = "ghp_" + "a" * 36
        escaped = value.replace("a" * 36, "\\u0061" * 36)
        for text in [value, '{"value":"' + escaped + '"}',
                     '{"value":"' + escaped + '","value":"clean"}']:
            with self.subTest(encoded=text != value), self.assertRaises(ValueError) as error:
                module.secret_check(text.encode())
            self.assertNotIn(value, str(error.exception))

    def test_recovery_scanner_covers_project_sanitizer_token_formats_and_json_escapes(self):
        values = [prefix + "A" * length for prefix, length in [
            ("github_pat_", 36), ("ghp_", 36), ("sk-proj-", 36), ("pat-", 36),
            ("AIza", 35), ("xoxb-", 36), ("Bearer ", 36)]]
        for value in values:
            for escaped in [False, True]:
                text = json.dumps({"fixture": value})
                if escaped:
                    text = text.replace("A", "\\u0041")
                with self.subTest(prefix=value.split("_")[0], escaped=escaped), self.assertRaises(ValueError) as error:
                    module.secret_check(text.encode())
                self.assertNotIn(value, str(error.exception))

    def test_bearer_documentation_does_not_block_historical_archive_recovery(self):
        for value in ["QR login obtains iLink Bearer bot_token", "Bearer Cloudflare Token"]:
            module.secret_check(value.encode())

    def test_short_authorization_header_values_are_rejected_in_all_schemes(self):
        for scheme in ["Bearer", "Token", "Basic"]:
            value = scheme + " abcdefgh"
            fixtures = ["Authorization: " + value, json.dumps({"Authorization": value})]
            fixtures.append(fixtures[-1].replace("abcdefgh", "\\u0061bcdefgh"))
            for fixture in fixtures:
                with self.subTest(scheme=scheme), self.assertRaises(ValueError) as error:
                    module.secret_check(fixture.encode())
                self.assertNotIn(value, str(error.exception))

    def test_opaque_sensitive_json_values_are_rejected_with_escaped_and_duplicate_keys(self):
        for key in ["api_key", "apiKey", "api-key", "access_token", "refreshToken", "password", "secret", "token", "x-api-key"]:
            value = "opaque-private-value"
            fixtures = [json.dumps({"nested": {key: value}}),
                        '{"' + key + '":"' + value + '","' + key + '":""}']
            fixtures.append(fixtures[0].replace(key[0], "\\u" + format(ord(key[0]), "04x"), 1))
            for fixture in fixtures:
                with self.subTest(key=key), self.assertRaises(ValueError) as error:
                    module.secret_check(fixture.encode())
                self.assertNotIn(value, str(error.exception))

    def test_empty_sensitive_fields_and_schema_arrays_are_not_credentials(self):
        for value in [None, "", False, []]:
            module.secret_check(json.dumps({"api_key": value}).encode())
        module.secret_check(json.dumps([["token", "schema example"]]).encode())

    def test_sealed_legacy_permission_notes_do_not_relax_other_credential_fields(self):
        note = json.dumps({"authorization": "Owner must approve this experiment"}).encode()
        with self.assertRaises(ValueError):
            module.secret_check(note)
        module.secret_check(note, permission_notes_allowed=True)
        for value in [{"authorization": "Bearer abcdefgh"},
                      {"authorization": "Permission pending", "api_key": "opaque-secret"}]:
            with self.assertRaises(ValueError):
                module.secret_check(json.dumps(value).encode(), permission_notes_allowed=True)

    def test_legacy_permission_note_allowance_expires_when_archive_bytes_change(self):
        archive = self.archive()
        path = archive / "data/2026/report.json"
        payload = json.loads(path.read_text())
        payload["authorization"] = "Owner must approve the synthetic experiment"
        data = json.dumps(payload).encode()
        for target in [path, archive / "LATEST.json"]:
            target.write_bytes(data)
        sealed = {module.digest(data)}
        self.assertEqual(module.archive_state(archive, sealed)["status"], "valid")
        payload["changes"] = ["unreviewed edit"]
        for target in [path, archive / "LATEST.json"]:
            target.write_text(json.dumps(payload))
        with self.assertRaisesRegex(ValueError, "credential field"):
            module.archive_state(archive, sealed)

    def test_invalid_timestamp_never_leaks_a_credential_through_parser_errors(self):
        archive = self.archive()
        path = archive / "data/2026/report.json"
        payload = json.loads(path.read_text())
        value = "ghp_" + "a" * 36
        payload["generated_at"] = value
        for escaped in [False, True]:
            text = json.dumps(payload)
            if escaped:
                text = text.replace("a" * 36, "\\u0061" * 36)
            path.write_bytes(text.encode())
            (archive / "LATEST.json").write_bytes(text.encode())
            with self.subTest(escaped=escaped), self.assertRaisesRegex(ValueError, "credential-like") as error:
                module.archive_state(archive)
            self.assertNotIn(value, str(error.exception))

    def test_invalid_generation_timestamp_never_echoes_arbitrary_input(self):
        archive = self.archive()
        payload = json.loads((archive / "LATEST.json").read_text())
        value = "private-context-that-is-not-a-date"
        payload["generated_at"] = value
        for path in [archive / "LATEST.json", archive / "data/2026/report.json"]:
            path.write_text(json.dumps(payload))
        with self.assertRaises(ValueError) as error:
            module.archive_state(archive)
        self.assertNotIn(value, str(error.exception))

    def test_recovery_preserves_allowlisted_progress_without_copying_arbitrary_state(self):
        for name in ["prepare.py", "control.py", "README.md"]:
            (self.runtime / name).write_text("fixture\n")
        worktree = self.root / "repo"
        for name in ["docs/architecture/agent/action-card-ledger.md", ".github/dependabot.yml"]:
            path = worktree / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n")
        self.control.state.mkdir()
        (self.control.state / "decision-queue.json").write_text('{"items":[]}')
        (self.control.state / "action-queue.json").write_text('{"cards":[]}')
        (self.control.state / "unrelated-private.json").write_text('"excluded"')
        runtime = Mock()
        runtime.manifest.return_value = (None, None, {"task_id": "code", "state": "prepared",
            "worktree": str(worktree), "baseline_sha": "a" * 40})
        with patch.object(self.control, "runtime", return_value=runtime):
            receipt = self.control.recovery(self.root / "run")
        records = json.loads((Path(receipt["backup"]) / "manifest.json").read_text())["files"]
        paths = {record["path"] for record in records}
        self.assertIn("runtime/control-state/decision-queue.json", paths)
        self.assertIn("runtime/control-state/action-queue.json", paths)
        self.assertFalse(any("unrelated-private" in path for path in paths))
        self.assertEqual(receipt["files"], receipt["restored_files"])

    def test_writer_lease_never_steals_another_owner_even_after_time_passes(self):
        runtime = Mock()
        runtime.manifest.side_effect = lambda p: (None, None, {"state": "prepared", "task_id": "code"})
        with patch.object(self.control, "runtime", return_value=runtime):
            self.assertTrue(self.control.lease(str(self.root / "one"))["ok"])
            self.assertFalse(self.control.lease(str(self.root / "two"))["ok"])
            self.assertFalse(self.control.lease(str(self.root / "two"), release=True)["ok"])
            self.assertTrue(self.control.lease(str(self.root / "one"), release=True)["ok"])
            self.assertTrue(self.control.lease(str(self.root / "two"))["ok"])

    def test_windows_lock_initializes_one_byte_and_unlocks_after_exception(self):
        backend = Mock(LK_NBLCK=1, LK_UNLCK=2)
        with patch.object(module, "msvcrt", backend), patch.object(module, "fcntl", None):
            with self.assertRaisesRegex(ValueError, "fixture"):
                with self.control.lock():
                    self.assertEqual((self.control.state / "control.lock").read_bytes(), b"\0")
                    raise ValueError("fixture")
        self.assertEqual([c.args[1:] for c in backend.locking.call_args_list], [(1, 1), (2, 1)])

    def test_closed_owner_can_release_without_reopening_missing_worktree(self):
        runtime = Mock()
        runtime.manifest.return_value = (None, None, {"state": "prepared", "task_id": "code"})
        owner = str(self.root / "one")
        with patch.object(self.control, "runtime", return_value=runtime):
            self.control.lease(owner)
            runtime.manifest.return_value = (None, None, {"state": "completed", "task_id": "code"})
            runtime.verify_worktree.reset_mock()
            self.assertTrue(self.control.lease(owner, release=True)["ok"])
            runtime.verify_worktree.assert_not_called()
            with self.assertRaises(ValueError):
                self.control.lease(owner)

    def test_observation_acknowledgement_rejects_forged_digest_and_foreign_scope(self):
        data = {"main": "a" * 40}
        with patch.object(self.control, "remote_observation", return_value=data):
            first = self.control.preflight("pr")
        self.assertTrue(first["changed"])
        self.control.acknowledge("pr", first["observation"], "waiting")
        with patch.object(self.control, "remote_observation", side_effect=AssertionError("unexpected API")):
            self.assertEqual(self.control.preflight("pr")["reason"], "backoff")
        path = Path(first["observation"])
        saved = json.loads(path.read_text())
        saved["data"] = {"main": "b" * 40}
        path.write_text(json.dumps(saved))
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.control.acknowledge("pr", path, "handled")
        with self.assertRaises(ValueError):
            self.control.acknowledge("code", path, "handled")

    def test_issue_observation_stores_comment_digest_without_body(self):
        raw = [{"number": 1, "updatedAt": "now", "comments": [{"id": "1", "body": "private text"}]}]
        self.control.policy["task_ids"].append("issue")
        with patch.object(module, "gh", side_effect=[raw, []]):
            result = self.control.remote_observation("issue")
        self.assertNotIn("private text", json.dumps(result))
        self.assertEqual(result["issues"][0]["comments"][0]["body_sha256"], module.digest("private text"))

    def test_issue_rechecks_when_a_pr_changes_without_an_issue_comment(self):
        self.control.policy["task_ids"].append("issue")
        issue = [{"number": 1, "updatedAt": "same", "comments": []}]
        opened = [{"number": 2, "state": "OPEN", "headRefOid": "a" * 40}]
        merged = [{"number": 2, "state": "MERGED", "headRefOid": "a" * 40}]
        with patch.object(module, "gh", side_effect=[issue, opened, issue, merged]):
            first = module.digest(self.control.remote_observation("issue"))
            second = module.digest(self.control.remote_observation("issue"))
        self.assertNotEqual(first, second)

    def test_unknown_task_has_no_side_effects(self):
        with self.assertRaises(ValueError):
            self.control.preflight("unapproved")
        self.assertFalse(self.control.state.exists())

    def test_registry_cannot_redirect_backups_to_an_unapproved_directory(self):
        approved = self.archive()
        self.control.policy["archive_directories"] = {"radar": str(approved)}
        self.control.registry = {"radar": {"archive_dir": str(self.root / "private")}}
        with self.assertRaisesRegex(ValueError, "approved task scope"):
            self.control.archive_directory("radar")

    def test_workflow_approval_changes_are_detected_without_any_check_runs(self):
        sha = "a" * 40
        self.control.policy["excluded_pr_authors"] = []
        repository = {"data": {"repository": {"defaultBranchRef": {"name": "main"},
            "pullRequests": {"pageInfo": {"hasNextPage": False}, "nodes": [{"number": 1,
                "headRefOid": sha, "author": {"login": "contributor"},
                "reviewThreads": {"pageInfo": {"hasNextPage": False}},
                "commits": {"nodes": [{"commit": {"statusCheckRollup": None}}]}}]}}}}
        status = {"conclusion": "action_required"}
        def response(args):
            if args[1] == "graphql":
                return repository
            if "/actions/runs?" in args[1]:
                return {"total_count": 1, "workflow_runs": [{"id": 1, "workflow_id": 2,
                    "head_sha": sha, "status": "completed", "created_at": "2026-10-01", **status}]}
            return []
        with patch.object(module, "gh", side_effect=response):
            first = module.digest(self.control.remote_observation("pr"))
            status["conclusion"] = "failure"
            second = module.digest(self.control.remote_observation("pr"))
        self.assertNotEqual(first, second)

    def test_all_ruleset_pages_participate_in_the_pr_fingerprint(self):
        self.control.policy["excluded_pr_authors"] = []
        repo = {"data": {"repository": {"defaultBranchRef": {"name": "main"},
            "pullRequests": {"pageInfo": {"hasNextPage": False}, "nodes": []}}}}
        second_page = {"id": 2, "enforcement": "active"}
        calls = []
        def response(args):
            calls.append(args)
            if args[1] == "graphql":
                return repo
            if "/rulesets?" in args[-1]:
                return [[{"id": 1}], [second_page.copy()]]
            return []
        with patch.object(module, "gh", side_effect=response):
            first = self.control.remote_observation("pr")
            second_page["enforcement"] = "disabled"
            second = self.control.remote_observation("pr")
        self.assertNotEqual(first["ruleset_digest"], second["ruleset_digest"])
        self.assertTrue(any("--paginate" in args and "--slurp" in args for args in calls))

    def test_all_effective_branch_rule_pages_participate_in_the_pr_fingerprint(self):
        self.control.policy["excluded_pr_authors"] = []
        repo = {"data": {"repository": {"defaultBranchRef": {"name": "main"},
            "pullRequests": {"pageInfo": {"hasNextPage": False}, "nodes": []}}}}
        second_page = {"type": "required_status_checks", "parameters": {"strict": True}}
        def response(args):
            if args[1] == "graphql":
                return repo
            if "/rules/branches/" in args[-1]:
                return [[{"type": "deletion"}], [second_page.copy()]]
            return [[]]
        with patch.object(module, "gh", side_effect=response):
            first = self.control.remote_observation("pr")
            second_page["parameters"] = {"strict": False}
            second = self.control.remote_observation("pr")
        self.assertNotEqual(first["protection_digest"], second["protection_digest"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
