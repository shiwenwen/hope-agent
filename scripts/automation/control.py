#!/usr/bin/env python3
"""Deterministic local operations for project automations; no model or write API calls."""
import argparse
import contextlib
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import uuid

try:
    import msvcrt
except ImportError:
    msvcrt = None
try:
    import fcntl
except ImportError:
    fcntl = None

REPOSITORY = "shiwenwen/hope-agent"
REQUIRED = {"task_name", "generated_at", "baseline", "scope", "sources", "changes",
            "decisions", "action_items", "evidence_gaps", "no_material_change"}
SECRETS = [re.compile(r"(?:sk|pat)-[A-Za-z0-9_-]{20,}|github_pat_[A-Za-z0-9_]{20,}|"
                      r"gh[pousr]_[A-Za-z0-9_]{20,}|AIza[A-Za-z0-9_-]{35}|xox[abp]-[A-Za-z0-9_-]{20,}"),
           re.compile(r"\bauthorization[\"']?\s*[:=,]\s*[\"']?(?:bearer|token|basic)\s+[^\s,'\")`]+", re.IGNORECASE),
           re.compile(r"\bbearer\s+[A-Za-z0-9._~+/=-]{20,}", re.IGNORECASE),
           re.compile(r"AKIA[0-9A-Z]{16}"),
           re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")]
SENSITIVE_KEYS = {"apikey", "accesstoken", "refreshtoken", "authorization",
                  "xapikey", "bearer", "password", "secret", "token"}


class JsonObjectPairs(list):
    """Tag object members without dropping duplicate keys or confusing them with arrays."""


def now():
    return dt.datetime.now(dt.timezone.utc)


def digest(value):
    data = value if isinstance(value, bytes) else json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return hashlib.sha256(data).hexdigest()


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".state-", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def safe_relative(root, relative):
    value = Path(relative)
    if value.is_absolute() or ".." in value.parts:
        raise ValueError("absolute or traversing artifact path")
    path = root / value
    for parent in [path, *path.parents]:
        if parent == root.parent:
            break
        if parent.is_symlink():
            raise ValueError("symlinked artifact path")
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError("artifact escaped its scope")
    return path


def secret_check(data, permission_notes_allowed=False):
    text = data.decode("utf-8")
    candidates = [text]
    try:
        # Preserve duplicate object values while decoding escapes for the security scan.
        decoded = json.loads(text, object_pairs_hook=JsonObjectPairs)
        def check_keys(value):
            if isinstance(value, JsonObjectPairs):
                for key, item in value:
                    normalized = key.casefold().replace("_", "").replace("-", "")
                    permission_note = normalized == "authorization" and permission_notes_allowed
                    if normalized in SENSITIVE_KEYS and not permission_note and item is not None and item is not False and item != "" and item != []:
                        raise ValueError("credential field content; raw value withheld")
                    check_keys(item)
            elif isinstance(value, list):
                for item in value:
                    check_keys(item)
        check_keys(decoded)
        candidates.append(json.dumps(decoded, ensure_ascii=False))
    except json.JSONDecodeError:
        pass
    if any(pattern.search(candidate) for pattern in SECRETS for candidate in candidates):
        raise ValueError("credential-like content; raw value withheld")


def command(args, cwd=None, timeout=90):
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, timeout=timeout,
                            env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
    if result.returncode:
        raise RuntimeError("command failed: " + args[0] + " (exit " +
                           str(result.returncode) + "); raw output withheld")
    return result.stdout.strip()


def gh(args):
    return json.loads(command(["gh", *args]))


def gh_paginated_array(endpoint):
    pages = gh(["api", "--paginate", "--slurp", endpoint])
    if not isinstance(pages, list) or any(not isinstance(page, list) for page in pages):
        raise RuntimeError("API array pagination returned an incomplete shape")
    return [record for page in pages for record in page]


def archive_state(directory, permission_note_documents=()):
    """Accept legacy indexes for reading, but require immutable targets and matching dates."""
    root = Path(directory).resolve()
    result = {"directory": str(root), "status": "missing"}
    latest_path = safe_relative(root, "LATEST.json")
    report_path = safe_relative(root, "LATEST.md")
    if not latest_path.is_file() or not report_path.is_file():
        return result
    latest_bytes = latest_path.read_bytes()
    report_bytes = report_path.read_bytes()
    secret_check(latest_bytes, digest(latest_bytes) in permission_note_documents)
    secret_check(report_bytes)
    latest = json.loads(latest_bytes)
    index = not REQUIRED.issubset(latest)
    if index:
        data = safe_relative(root, latest["data"])
        report = safe_relative(root, latest["report"])
    else:
        def matches_in(folder, suffix, content):
            matches = []
            base = safe_relative(root, folder)
            for year in base.glob("[0-9][0-9][0-9][0-9]"):
                safe_relative(root, year.relative_to(root))
                if not year.is_dir():
                    continue
                for candidate in year.glob("*" + suffix):
                    safe_relative(root, candidate.relative_to(root))
                    if candidate.is_file() and candidate.read_bytes() == content:
                        matches.append(candidate)
            return matches
        matches = matches_in("data", ".json", latest_bytes)
        reports = matches_in("reports", ".md", report_bytes)
        if len(matches) != 1 or len(reports) != 1:
            raise ValueError("LATEST does not match exactly one immutable pair")
        data, report = matches[0], reports[0]
    for path, folder, suffix in [(data, "data", ".json"), (report, "reports", ".md")]:
        parts = path.relative_to(root).parts
        if len(parts) != 3 or parts[0] != folder or not re.fullmatch(r"\d{4}", parts[1]) or path.suffix != suffix:
            raise ValueError("archive target is outside the immutable year layout")
    payload_bytes, markdown = data.read_bytes(), report.read_bytes()
    if data.stem != report.stem or data.parent.name != report.parent.name:
        raise ValueError("LATEST contains a mixed immutable report/data pair")
    secret_check(payload_bytes, digest(payload_bytes) in permission_note_documents)
    secret_check(markdown)
    payload = json.loads(payload_bytes)
    missing = REQUIRED - set(payload)
    if missing:
        raise ValueError("immutable data is missing required fields: " + ",".join(sorted(missing)))
    if latest.get("generated_at") != payload["generated_at"]:
        raise ValueError("index and immutable generation times differ")
    generated_at = payload["generated_at"]
    if not isinstance(generated_at, str):
        raise ValueError("generated_at must be an ISO timestamp")
    try:
        generated = dt.datetime.fromisoformat(generated_at.replace("Z", "+00:00"))
    except ValueError:
        raise ValueError("generated_at is not a valid ISO timestamp; raw value withheld") from None
    if str(generated.year) != data.parent.name:
        raise ValueError("immutable directory year differs from generated_at")
    if not isinstance(payload["sources"], list) or not isinstance(payload["action_items"], list):
        raise ValueError("invalid sources or action_items schema")
    result.update(status="legacy-index" if index else "valid", generated_at=payload["generated_at"],
                  data=str(data), report=str(report), data_sha256=digest(payload_bytes),
                  report_sha256=digest(markdown), action_items=len(payload["action_items"]))
    return result


class Control:
    def __init__(self, root=None, codex_home=None):
        self.root = Path(root or Path.home() / ".codex/automation-runtime/hope-agent").resolve()
        self.codex = Path(codex_home or Path.home() / ".codex").resolve()
        self.state = self.root / "control-state"
        self.policy = json.loads((self.root / "policy.json").read_text())
        self.registry = json.loads((self.root / "tasks.json").read_text())

    @contextlib.contextmanager
    def lock(self):
        self.state.mkdir(parents=True, exist_ok=True)
        with (self.state / "control.lock").open("a+") as stream:
            if msvcrt is not None:
                if os.fstat(stream.fileno()).st_size == 0:
                    stream.write("\0")
                    stream.flush()
                stream.seek(0)
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            elif fcntl is not None:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            else:
                raise RuntimeError("no supported file locking backend")
            try:
                yield
            finally:
                if msvcrt is not None:
                    stream.seek(0)
                    msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    fcntl.flock(stream, fcntl.LOCK_UN)

    def task(self, task):
        if not re.fullmatch(r"[a-z0-9-]+", task) or task not in self.policy["task_ids"]:
            raise ValueError("task is outside the approved project scope")
        return task

    def archive_directory(self, task):
        self.task(task)
        configured = Path(self.registry[task]["archive_dir"]).resolve()
        approved = self.policy.get("archive_directories", {}).get(task)
        if not approved or configured != Path(approved).resolve():
            raise ValueError("archive directory is outside the approved task scope")
        return configured

    def automations(self):
        database = self.codex / "sqlite/codex-dev.db"
        with sqlite3.connect("file:" + str(database) + "?mode=ro", uri=True) as conn:
            conn.row_factory = sqlite3.Row
            fields = ["id", "name", "status", "kind", "model", "reasoning_effort", "project_id",
                      "target_thread_id", "last_run_at", "next_run_at", "next_run_nominal_at"]
            return [{key: row[key] for key in fields}
                    for row in conn.execute("SELECT * FROM automations")
                    if row["id"] in self.policy["task_ids"]]

    def snapshot(self):
        archives, incomplete = [], []
        for key in self.policy["task_ids"]:
            task = self.registry.get(key, {})
            if task.get("archive_dir"):
                try:
                    state = archive_state(self.archive_directory(key), self.policy.get("permission_note_document_sha256", []))
                except (OSError, ValueError, KeyError) as error:
                    state = {"status": "invalid", "error": str(error)}
                archives.append({"task_id": key, **state})
            for path in (self.root / "runs" / key).glob("*/run.json"):
                run = json.loads(path.read_text())
                if run.get("state") in {"prepared", "retained"}:
                    incomplete.append({"task_id": key, "run_directory": str(path.parent),
                                       "state": run["state"], "baseline_sha": run["baseline_sha"],
                                       "verified_at": run["verified_at"]})
        database = self.codex / "sqlite/codex-dev.db"
        with sqlite3.connect("file:" + str(database) + "?mode=ro", uri=True) as conn:
            conn.row_factory = sqlite3.Row
            runs = [dict(row) for row in conn.execute(
                "SELECT automation_id,thread_id,status,created_at,updated_at FROM automation_runs "
                "ORDER BY created_at DESC") if row["automation_id"] in self.policy["task_ids"]]
        latest = {}
        for run in runs:
            latest.setdefault(run["automation_id"], run)
        result = {"schema_version": "hope-automation-health.v1", "observed_at": now().isoformat(),
                  "automations": self.automations(), "archives": archives,
                  "incomplete_environments": incomplete, "latest_run_records": list(latest.values()),
                  "engine_status": "requires read_thread; inbox status is not execution success"}
        atomic_json(self.state / "health.json", result)
        return result

    def normalize_archives(self):
        promoted = []
        with self.lock():
            for key in self.policy["task_ids"]:
                item = self.registry.get(key, {})
                if not item.get("archive_dir"):
                    continue
                state = archive_state(self.archive_directory(key), self.policy.get("permission_note_document_sha256", []))
                if state["status"] != "legacy-index":
                    continue
                directory = Path(state["directory"])
                backup = self.root / "backups" / ("latest-index-" + uuid.uuid4().hex) / key
                backup.mkdir(parents=True)
                for name in ["LATEST.md", "LATEST.json"]:
                    shutil.copy2(directory / name, backup / name)
                staged = []
                try:
                    for name, source in [("LATEST.md", state["report"]),
                                         ("LATEST.json", state["data"])]:
                        fd, temp = tempfile.mkstemp(prefix=".promote-", dir=str(directory))
                        with os.fdopen(fd, "wb") as stream:
                            stream.write(Path(source).read_bytes())
                            stream.flush()
                            os.fsync(stream.fileno())
                        staged.append((Path(temp), directory / name))
                    for temp, target in staged:
                        os.replace(temp, target)
                    verified = archive_state(directory, self.policy.get("permission_note_document_sha256", []))
                    if verified["status"] != "valid":
                        raise ValueError("promotion did not produce a complete pair")
                except Exception:
                    for name in ["LATEST.md", "LATEST.json"]:
                        shutil.copy2(backup / name, directory / name)
                    raise
                finally:
                    for temp, _ in staged:
                        temp.unlink(missing_ok=True)
                promoted.append({"task_id": key, "backup": str(backup), **verified})
        return {"promoted": promoted}

    def runtime(self):
        path = self.root / "prepare.py"
        spec = importlib.util.spec_from_file_location("managed_prepare", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.Runtime(self.root)

    def lease(self, run_directory, release=False):
        """No timeout stealing: dead owners must be inspected before releasing their lease."""
        runtime = self.runtime()
        _, _, run = runtime.manifest(run_directory)
        if run["state"] not in {"prepared", "retained"} and not release:
            raise ValueError("closed run cannot acquire a writer lease")
        self.task(run["task_id"])
        if run["state"] != "completed":
            runtime.verify_worktree(run)
        path = self.state / "writer-lease.json"
        owner = str(Path(run_directory).resolve())
        with self.lock():
            prior = json.loads(path.read_text()) if path.exists() else None
            if prior and prior["owner"] != owner:
                return {"ok": False, "reason": "writer_busy", "owner": prior["owner"]}
            if release:
                if not prior:
                    return {"ok": True, "released": False}
                path.unlink()
                return {"ok": True, "released": True}
            atomic_json(path, {"owner": owner, "task_id": run["task_id"],
                               "acquired_at": prior["acquired_at"] if prior else now().isoformat()})
            return {"ok": True, "owner": owner}

    def remote_observation(self, task):
        self.task(task)
        if task == "pr":
            query = """query($owner:String!,$name:String!,$after:String){repository(owner:$owner,name:$name){
              defaultBranchRef{name target{... on Commit{oid}}}
              pullRequests(first:100,states:OPEN,after:$after){pageInfo{hasNextPage endCursor} nodes{
                number headRefOid baseRefOid updatedAt isDraft mergeable mergeStateStatus reviewDecision
                author{login} comments(last:1){totalCount nodes{id updatedAt}}
                reviews(last:1){totalCount nodes{id submittedAt state}}
                reviewThreads(first:100){pageInfo{hasNextPage} nodes{id isResolved isOutdated}}
                commits(last:1){nodes{commit{statusCheckRollup{contexts(first:100){
                  pageInfo{hasNextPage} nodes{... on CheckRun{name status conclusion}
                  ... on StatusContext{context state}}}}}}}
              }}}}"""
            prs, after, base = [], None, None
            while True:
                args = ["api", "graphql", "-f", "query=" + query, "-f", "owner=shiwenwen",
                        "-f", "name=hope-agent"]
                if after:
                    args.extend(["-f", "after=" + after])
                page = gh(args)
                if page.get("errors"):
                    raise RuntimeError("GitHub observation is incomplete")
                repo = page["data"]["repository"]
                base = repo["defaultBranchRef"]
                for pr in repo["pullRequests"]["nodes"]:
                    if (pr.get("author") or {}).get("login") in self.policy["excluded_pr_authors"]:
                        continue
                    if pr["reviewThreads"]["pageInfo"]["hasNextPage"]:
                        raise RuntimeError("review thread observation needs pagination; do not skip")
                    for item in pr["commits"]["nodes"]:
                        rollup = item["commit"]["statusCheckRollup"]
                        if rollup and rollup["contexts"]["pageInfo"]["hasNextPage"]:
                            raise RuntimeError("check observation needs pagination; do not skip")
                    prs.append(pr)
                info = repo["pullRequests"]["pageInfo"]
                if not info["hasNextPage"]:
                    break
                after = info["endCursor"]
            protections = gh_paginated_array("repos/" + REPOSITORY + "/rules/branches/main")
            rules = gh_paginated_array("repos/" + REPOSITORY + "/rulesets?includes_parents=true")
            workflows = {}
            for sha in sorted({pr["headRefOid"] for pr in prs}):
                # A workflow can change approval status without creating any check runs.
                page = gh(["api", "repos/" + REPOSITORY + "/actions/runs?head_sha=" + sha + "&per_page=100"])
                if page.get("total_count", 0) > 100:
                    raise RuntimeError("workflow observation needs pagination; do not skip")
                latest = {}
                for run in page["workflow_runs"]:
                    if run["head_sha"] != sha:
                        raise RuntimeError("workflow response is not bound to the expected head")
                    key = str(run["workflow_id"])
                    if key not in latest or run["created_at"] > latest[key]["created_at"]:
                        latest[key] = {field: run.get(field) for field in ["id", "workflow_id", "head_sha",
                            "status", "conclusion", "run_attempt", "created_at"]}
                workflows[sha] = latest
            return {"main": base, "pull_requests": sorted(prs, key=lambda x: x["number"]),
                    "workflow_runs": workflows, "protection_digest": digest(protections),
                    "ruleset_digest": digest(rules)}
        if task == "issue":
            issues = gh(["issue", "list", "--repo", REPOSITORY, "--state", "open", "--limit", "1000",
                         "--json", "number,updatedAt,comments"])
            # A related PR can merge or get new feedback without touching the Issue.
            prs = gh(["pr", "list", "--repo", REPOSITORY, "--state", "all", "--limit", "1000",
                      "--json", "number,headRefOid,baseRefOid,state,updatedAt"])
            if len(issues) >= 1000 or len(prs) >= 1000:
                raise RuntimeError("Issue/PR observation needs pagination; do not skip")
            summaries = [{"number": issue["number"], "updatedAt": issue["updatedAt"],
                     "comments": [{"id": comment.get("id"), "updatedAt": comment.get("updatedAt"),
                                   "createdAt": comment.get("createdAt"),
                                   "body_sha256": digest(comment.get("body", ""))}
                                  for comment in issue["comments"]]} for issue in issues]
            return {"issues": sorted(summaries, key=lambda x: x["number"]),
                    "pull_requests": sorted(prs, key=lambda x: x["number"])}
        if task == "main":
            return {"main": gh(["api", "repos/" + REPOSITORY + "/commits/main"])["sha"],
                    "releases": gh(["release", "list", "--repo", REPOSITORY, "--limit", "30",
                                    "--json", "tagName,isDraft,isPrerelease,publishedAt"]),
                    "release_prs": gh(["pr", "list", "--repo", REPOSITORY, "--state", "open",
                                       "--search", "head:chore/release", "--limit", "100",
                                       "--json", "number,headRefOid,updatedAt"]),
                    "release_runs": gh(["run", "list", "--repo", REPOSITORY, "--workflow", "release.yml",
                                        "--limit", "8", "--json", "databaseId,headSha,status,conclusion"])}
        raise ValueError("remote preflight is not registered for this task")

    def preflight(self, task, force=False):
        self.task(task)
        saved = self.state / "observations" / (task + ".json")
        prior = json.loads(saved.read_text()) if saved.exists() else {}
        if not force and prior.get("next_probe_at") and now() < dt.datetime.fromisoformat(prior["next_probe_at"]):
            return {"changed": False, "reason": "backoff", "next_probe_at": prior["next_probe_at"]}
        data = self.remote_observation(task)
        fingerprint = digest(data)
        path = self.state / "observations" / task / (now().strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8] + ".json")
        atomic_json(path, {"task_id": task, "observed_at": now().isoformat(),
                           "fingerprint": fingerprint, "data": data})
        return {"changed": fingerprint != prior.get("fingerprint"),
                "observation": str(path), "fingerprint": fingerprint}

    def acknowledge(self, task, observation, outcome):
        self.task(task)
        path = Path(observation).resolve()
        if not path.is_relative_to(self.state / "observations" / task):
            raise ValueError("observation belongs to another scope")
        source = json.loads(path.read_text())
        if source["task_id"] != task or digest(source["data"]) != source["fingerprint"]:
            raise ValueError("observation identity or digest mismatch")
        if outcome not in {"no-change", "waiting", "blocked", "handled"}:
            raise ValueError("not a supported observation outcome; no fabricated delivery state")
        target = self.state / "observations" / (task + ".json")
        with self.lock():
            prior = json.loads(target.read_text()) if target.exists() else {}
            if prior.get("observed_at", "") > source["observed_at"]:
                raise ValueError("refusing stale observation acknowledgement")
            hours = 6 if outcome in {"no-change", "waiting", "blocked"} else 2
            atomic_json(target, {**source, "outcome": outcome, "acknowledged_at": now().isoformat(),
                                 "next_probe_at": (now() + dt.timedelta(hours=hours)).isoformat(),
                                 "note": "observation handled, not business delivery success"})
        return {"ok": True, "outcome": outcome}

    def recovery(self, run_directory):
        runtime = self.runtime()
        _, _, run = runtime.manifest(run_directory)
        self.task(run["task_id"])
        runtime.verify_worktree(run)
        worktree = Path(run["worktree"])
        files = {}
        for name in ["policy.json", "tasks.json", "prepare.py", "control.py", "README.md"]:
            path = safe_relative(self.root, name)
            files["runtime/" + name] = path
        # Reconstructable health scans are excluded; preserve only owned progress contracts.
        for name in ["decision-queue.json", "action-queue.json", "dependency-status.json",
                     "regression-status.json", "retry-state.json"]:
            path = safe_relative(self.root, "control-state/" + name)
            if path.is_file():
                files["runtime/control-state/" + name] = path
        for task in self.policy["task_ids"]:
            path = safe_relative(self.codex, "automations/" + task + "/automation.toml")
            if path.exists():
                files["automations/" + task + "/automation.toml"] = path
            item = self.registry.get(task, {})
            if item.get("archive_dir"):
                archive = self.archive_directory(task)
                for folder, suffix in [("reports", ".md"), ("data", ".json"), ("timestamps", ".json")]:
                    for source in (archive / folder).rglob("*" + suffix):
                        relative = source.relative_to(archive)
                        files["archives/" + task + "/" + str(relative)] = safe_relative(archive, relative)
                for name in ["LATEST.md", "LATEST.json"]:
                    if (archive / name).exists():
                        files["archives/" + task + "/" + name] = safe_relative(archive, name)
        for name in ["docs/architecture/agent/action-card-ledger.md", ".github/dependabot.yml"]:
            files["project/" + name] = safe_relative(worktree, name)
        destination = self.root / "recovery" / (now().strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8])
        destination.mkdir(parents=True)
        self.state.mkdir(parents=True, exist_ok=True)
        started, records = now(), []
        try:
            for relative, source in sorted(files.items()):
                data = source.read_bytes()
                secret_check(data, digest(data) in self.policy.get("permission_note_document_sha256", []))
                target = safe_relative(destination, relative)
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open("xb") as stream:
                    stream.write(data)
                records.append({"path": relative, "sha256": digest(data), "bytes": len(data)})
            manifest = {"schema_version": "hope-automation-recovery.v1", "created_at": now().isoformat(),
                        "baseline_sha": run["baseline_sha"], "files": records,
                        "scope": "project operations and allowlisted progress contracts only; no user database, credentials, git metadata or caches"}
            atomic_json(destination / "manifest.json", manifest)
            with tempfile.TemporaryDirectory(prefix="restore-", dir=str(self.state)) as temp:
                self.verify_restore(destination, Path(temp), manifest)
            receipt = {"ok": True, "backup": str(destination), "files": len(records),
                       "restored_files": len(records), "manifest_sha256": digest((destination / "manifest.json").read_bytes()),
                       "elapsed_seconds": round((now() - started).total_seconds(), 3),
                       "offsite_verified": False}
            atomic_json(destination / "restore-receipt.json", receipt)
            atomic_json(self.state / "last-recovery.json", receipt)
            return receipt
        except Exception:
            # Keep a partial backup for diagnosis, never promote it as valid recovery material.
            atomic_json(destination / "failed.json", {"state": "failed", "completed_files": len(records)})
            raise

    @staticmethod
    def verify_restore(source, target, manifest):
        restored = 0
        for record in manifest["files"]:
            original = safe_relative(source, record["path"])
            destination = safe_relative(target, record["path"])
            data = original.read_bytes()
            if len(data) != record["bytes"] or digest(data) != record["sha256"]:
                raise ValueError("backup content digest mismatch")
            destination.parent.mkdir(parents=True, exist_ok=True)
            with destination.open("xb") as stream:
                stream.write(data)
            if digest(destination.read_bytes()) != record["sha256"]:
                raise ValueError("restored content digest mismatch")
            restored += 1
        return restored


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root")
    parser.add_argument("--codex-home")
    commands = parser.add_subparsers(dest="operation", required=True)
    commands.add_parser("snapshot")
    commands.add_parser("normalize-archives")
    p = commands.add_parser("lease")
    p.add_argument("run_directory")
    p.add_argument("--release", action="store_true")
    p = commands.add_parser("preflight")
    p.add_argument("task")
    p.add_argument("--force", action="store_true")
    p = commands.add_parser("acknowledge")
    p.add_argument("task")
    p.add_argument("observation")
    p.add_argument("outcome", choices=["no-change", "waiting", "blocked", "handled"])
    p = commands.add_parser("recovery")
    p.add_argument("run_directory")
    args = parser.parse_args()
    try:
        control = Control(args.root, args.codex_home)
        if args.operation == "snapshot":
            result = control.snapshot()
        elif args.operation == "normalize-archives":
            result = control.normalize_archives()
        elif args.operation == "lease":
            result = control.lease(args.run_directory, args.release)
        elif args.operation == "preflight":
            result = control.preflight(args.task, args.force)
        elif args.operation == "acknowledge":
            result = control.acknowledge(args.task, args.observation, args.outcome)
        else:
            result = control.recovery(args.run_directory)
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0 if result.get("ok", True) else 2
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"ok": False, "error": type(error).__name__, "message": str(error)}, ensure_ascii=False))
        return 2


if __name__ == "__main__":
    sys.exit(main())
