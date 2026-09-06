#!/usr/bin/env python3
"""Prepare and install one Hope Agent skill. Python 3.9+, standard library only.

Network acquisition uses Hope's bounded public GitHub reader. File metadata is
checked before any package bytes are requested; no repository code is executed.
The caller runs this helper through Hope Agent's normal exec permission/sandbox
boundary. Local checkouts are supported for other hosts and private repositories.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path, PurePosixPath
from urllib.parse import quote, unquote, urlsplit


MAX_FILES = 512
MAX_FILE_BYTES = 8 * 1024 * 1024
MAX_TOTAL_BYTES = 32 * 1024 * 1024
MAX_SKILL_BYTES = 64 * 1024
MAX_METADATA_BYTES = 2 * 1024 * 1024
MAX_METADATA_TOTAL_BYTES = 8 * 1024 * 1024
FETCH_PREFIX = b"hope-skill-fetch-v1\n"
RECEIPT = ".hope-skill-install.json"
SNAPSHOT_PATH = Path("review") / "payload"
SKIP_PARTS = {".git", ".hg", ".svn", "__pycache__", "node_modules"}
NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")


class InstallError(Exception):
    """An actionable failure whose message contains no downloaded content."""


def digest(value: object) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(encoded.encode()).hexdigest()


def is_link(info: os.stat_result) -> bool:
    # Windows junctions/reparse points must not bypass symlink rejection.
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, "st_file_attributes", 0) & 0x400)


def check_components(path: Path) -> None:
    for component in reversed([path, *path.parents]):
        try:
            info = component.lstat()
        except FileNotFoundError:
            continue
        if is_link(info):
            raise InstallError("Symlink or reparse point in installation path")
        if not stat.S_ISDIR(info.st_mode):
            raise InstallError("Installation path contains a non-directory")


def relative_path(raw: str, *, allow_root: bool = False) -> str:
    if not isinstance(raw, str):
        raise InstallError("Package paths must be strings")
    if allow_root and raw == ".":
        return raw
    parts = raw.split("/")
    if (not raw or any(p in {"", ".", ".."} for p in parts)
            or any(ord(c) < 32 for c in raw) or "\\" in raw or ":" in raw
            or raw.startswith("/") or any(p.endswith((".", " ")) for p in parts)):
        raise InstallError("Expected a relative package path without traversal or platform aliases")
    # Reject Windows device names on every platform to keep packages portable.
    if any(re.fullmatch(r"(?i)(con|prn|aux|nul|com[1-9]|lpt[1-9])", p.split(".")[0]) for p in parts):
        raise InstallError("Package contains a reserved device name")
    return raw


def skip_file(path: str) -> bool:
    parts = PurePosixPath(path).parts
    return (bool(set(parts) & SKIP_PARTS) or parts[-1] in {".DS_Store", RECEIPT}
            or parts[-1].endswith(".pyc"))


def read_regular(path: Path, limit: int = MAX_FILE_BYTES) -> tuple[bytes, os.stat_result]:
    before = path.lstat()
    if is_link(before) or not stat.S_ISREG(before.st_mode):
        raise InstallError("Expected a regular file, not a link or special file")
    # NONBLOCK prevents a concurrent regular-file-to-FIFO swap from hanging.
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    with os.fdopen(os.open(path, flags), "rb") as handle:
        opened = os.fstat(handle.fileno())
        if (not stat.S_ISREG(opened.st_mode) or is_link(opened)
                or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise InstallError("Source changed while reading; prepare a new snapshot")
        content = handle.read(limit + 1)
    if len(content) > limit:
        raise InstallError("Package contains an oversized file")
    return content, opened


def identity(content: bytes, *, allow_inactive: bool = False) -> dict[str, str]:
    """Validate portable identity fields, matching Hope's scalar conventions.

    This is deliberately not a general YAML parser. Vendor extensions remain
    byte-for-byte intact; the native `skill action=inspect` checks runtime loading.
    """
    if len(content) > MAX_SKILL_BYTES:
        raise InstallError("SKILL.md exceeds the 64 KiB installation limit")
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError:
        raise InstallError("SKILL.md must be UTF-8") from None
    lines = text.splitlines()
    if not lines or lines[0] != "---" or "---" not in lines[1:]:
        raise InstallError("SKILL.md requires delimited frontmatter")
    end = lines.index("---", 1)
    if not "\n".join(lines[end + 1:]).strip():
        raise InstallError("SKILL.md has no instructions")
    fields: dict[str, str] = {}
    for index in range(1, end):
        line = lines[index]
        if not line.strip() or line.startswith((" ", "\t", "#")):
            continue
        match = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_-]*):\s*(.*)", line)
        if not match:
            raise InstallError("Unsupported top-level frontmatter; use plain mapping keys")
        key, value = match.groups()
        if key in fields:
            raise InstallError("Duplicate frontmatter field")
        value = value.strip()
        if value in {"|", ">"}:
            continuation = []
            for nested in lines[index + 1:end]:
                if nested and not nested.startswith((" ", "\t")):
                    break
                continuation.append(nested.strip())
            value = (" " if value == ">" else "\n").join(continuation).strip()
        elif value.startswith(("'", '"')):
            if len(value) < 2 or value[-1] != value[0]:
                raise InstallError("Identity fields require complete single-line quoted scalars")
            value = value[1:-1]
        fields[key] = value
    name = fields.get("name", "")
    if not NAME.fullmatch(name) or len(name) > 64:
        raise InstallError("Skill name must be 1-64 lowercase letters, digits, and single hyphens")
    description = fields.get("description", "").strip()
    if (not description or len(description) > 1024 or description in {"null", "~", "true", "false"}
            or description.startswith(("[", "{", "&", "*", "!", "|", ">"))):
        raise InstallError("Skill description must be a nonempty plain, quoted, or |/> block scalar")
    if not allow_inactive and fields.get("status", "active").lower() != "active":
        raise InstallError("Draft or archived skills must be reviewed with ha-skill-creator first")
    return {"name": name, "description": description}


def inventory(root: Path) -> list[dict]:
    """Hash bounded regular files; reject links, special files and case aliases."""
    if is_link(root.lstat()) or not root.is_dir():
        raise InstallError("Skill source must be a real directory")
    files = []
    seen = set()
    total = 0
    entries = 0
    def walk_error(_error: OSError) -> None:
        raise InstallError("Cannot read a package directory; no complete snapshot was prepared")

    for base, dirs, names in os.walk(root, followlinks=False, onerror=walk_error):
        dirs.sort()
        names.sort()
        for name in [*dirs, *names]:
            item = Path(base) / name
            rel = relative_path(item.relative_to(root).as_posix())
            if skip_file(rel):
                if name in dirs:
                    dirs.remove(name)
                continue
            entries += 1
            if entries > MAX_FILES * 2 or rel.casefold() in seen:
                raise InstallError("Too many package entries or case-insensitive path collision")
            seen.add(rel.casefold())
            info = item.lstat()
            if is_link(info):
                raise InstallError("Skill packages cannot contain symlinks or reparse points")
            if stat.S_ISDIR(info.st_mode):
                continue
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_FILE_BYTES:
                raise InstallError("Package contains a special file or oversized file")
            content, info = read_regular(item, MAX_FILE_BYTES)
            total += len(content)
            if total > MAX_TOTAL_BYTES or len(files) >= MAX_FILES:
                raise InstallError("Skill exceeds 512 files or 32 MiB")
            files.append({"path": rel, "bytes": len(content),
                          "sha256": hashlib.sha256(content).hexdigest(),
                          "executable": bool(info.st_mode & 0o111)})
    if not any(item["path"] == "SKILL.md" for item in files):
        raise InstallError("No SKILL.md at the selected directory; select the skill subdirectory")
    return sorted(files, key=lambda item: item["path"])


def copy_snapshot(source: Path, target: Path, files: list[dict]) -> None:
    target.mkdir()
    for entry in files:
        rel = entry["path"]
        destination = target / rel
        destination.parent.mkdir(parents=True, exist_ok=True)
        content, _ = read_regular(source / rel, MAX_FILE_BYTES)
        if len(content) != entry["bytes"] or hashlib.sha256(content).hexdigest() != entry["sha256"]:
            raise InstallError("Package changed after preview; prepare again")
        with destination.open("xb") as handle:
            handle.write(content)
        destination.chmod(0o755 if entry["executable"] else 0o644)


def github_source(repo: str | None, url: str | None, path: str | None, ref: str | None) -> dict:
    if url:
        parsed = urlsplit(url)
        if (parsed.scheme != "https" or parsed.netloc != "github.com"
                or parsed.query or parsed.fragment):
            raise InstallError("Use an HTTPS github.com URL without credentials, ports, query, or fragment")
        parts = unquote(parsed.path).strip("/").split("/")
        if len(parts) < 2:
            raise InstallError("GitHub URL must identify a repository")
        repo = "/".join(parts[:2])
        if len(parts) > 2:
            if parts[2] not in {"tree", "blob"} or len(parts) < 4:
                raise InstallError("Expected a GitHub repository, tree, or SKILL.md blob URL")
            # A slash-containing ref must be supplied explicitly, never guessed.
            selected_ref = ref or parts[3]
            tail = "/".join(parts[3:])
            if not (tail == selected_ref or tail.startswith(selected_ref + "/")):
                raise InstallError("--ref does not match the GitHub URL")
            url_path = tail[len(selected_ref):].lstrip("/") or "."
            if parts[2] == "blob":
                if PurePosixPath(url_path).name != "SKILL.md":
                    raise InstallError("Blob URL must point to SKILL.md")
                url_path = str(PurePosixPath(url_path).parent)
            if path is not None and path != url_path:
                raise InstallError("--path conflicts with the GitHub URL")
            ref, path = selected_ref, url_path
    repo = (repo or "").removesuffix(".git")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+", repo) or repo.split("/")[-1] in {".", ".."}:
        raise InstallError("Expected a public GitHub owner/repository")
    ref = ref or "HEAD"
    if (not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,199}", ref)
            or ".." in ref or "//" in ref or ref.endswith(("/", ".", ".lock"))):
        raise InstallError("Invalid Git ref; supply a branch, tag, or commit")
    return {"kind": "github", "repo": repo, "ref": ref,
            "path": relative_path(path or ".", allow_root=True)}


def github_read(url: str, limit: int, deadline: float) -> bytes:
    """Use the owning binary inside exec, with its shared SSRF and body caps."""
    executable = os.environ.get("HOPE_AGENT_EXECUTABLE") or shutil.which("hope-agent")
    if not executable or not Path(executable).is_absolute():
        raise InstallError("GitHub acquisition requires Hope's native reader; run through Hope exec or use --local")
    remaining_ms = min(180_000, int((deadline - time.monotonic()) * 1000))
    if remaining_ms <= 0:
        raise InstallError("GitHub acquisition exceeded 180 seconds")
    command = [executable, "skill-source-fetch", "--url", url, "--max-bytes", str(limit),
               "--timeout-ms", str(remaining_ms)]
    try:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                                timeout=remaining_ms / 1000 + 1, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise InstallError("GitHub reader unavailable or acquisition timed out; use --local if needed") from None
    if result.returncode:
        if any(code in result.stderr for code in [b"http_status_403", b"http_status_429"]):
            raise InstallError("GitHub access denied or rate-limited; retry later or use --local")
        raise InstallError("GitHub acquisition failed; check public repo/ref/path and response size limits")
    if not result.stdout.startswith(FETCH_PREFIX) or len(result.stdout) > len(FETCH_PREFIX) + limit:
        raise InstallError("Incompatible or oversized response from Hope's GitHub reader")
    return result.stdout[len(FETCH_PREFIX):]


def object_id(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-f0-9]{40}", value):
        raise InstallError("GitHub metadata lacks an immutable object ID")
    return value


def acquire_github(source: dict, payload: Path) -> dict:
    deadline = time.monotonic() + 180
    api = "https://api.github.com/repos/" + source["repo"]
    metadata_bytes = 0

    def metadata(url: str, limit: int = MAX_METADATA_BYTES) -> bytes:
        nonlocal metadata_bytes
        remaining = MAX_METADATA_TOTAL_BYTES - metadata_bytes
        if remaining <= 0:
            raise InstallError("GitHub metadata exceeds acquisition limits")
        content = github_read(url, min(limit, remaining), deadline)
        metadata_bytes += len(content)
        return content

    commit = object_id(metadata(api + "/commits/" + quote(source["ref"], safe=""), 128)
                       .decode("ascii").strip())

    def tree(oid: str, recursive: bool = False) -> list[dict]:
        response = json.loads(metadata(api + "/git/trees/" + oid + ("?recursive=1" if recursive else "")))
        if not isinstance(response, dict) or response.get("truncated") is not False:
            raise InstallError("GitHub returned an incomplete file tree; no package files were downloaded")
        object_id(response.get("sha"))
        entries = response.get("tree")
        if not isinstance(entries, list) or any(not isinstance(entry, dict) for entry in entries):
            raise InstallError("GitHub returned an invalid file tree")
        return entries

    parts = [] if source["path"] == "." else source["path"].split("/")
    if len(parts) > 32:
        raise InstallError("GitHub skill path is too deep")
    oid = commit
    # Descend using non-recursive metadata so a large repository does not force
    # downloading its entire tree or blobs just to select one skill directory.
    for part in parts:
        matches = [entry for entry in tree(oid) if entry.get("path") == part]
        if len(matches) != 1 or matches[0].get("type") != "tree" or matches[0].get("mode") != "040000":
            raise InstallError("GitHub skill path must identify a directory without links or submodules")
        oid = object_id(matches[0].get("sha"))

    selected = []
    total = 0
    seen = set()
    for entry in tree(oid, recursive=True):
        rel = relative_path(entry["path"])
        if skip_file(rel):
            continue
        if rel.casefold() in seen or len(seen) >= MAX_FILES * 2:
            raise InstallError("Too many package entries or case-insensitive path collision")
        seen.add(rel.casefold())
        if entry.get("type") == "tree" and entry.get("mode") == "040000":
            continue
        if entry.get("type") != "blob" or entry.get("mode") not in {"100644", "100755"}:
            raise InstallError("GitHub skill contains a symlink, submodule, or special file")
        size = entry.get("size")
        if type(size) is not int or size < 0:
            raise InstallError("GitHub file metadata lacks a valid size")
        total += size
        limit = MAX_SKILL_BYTES if rel == "SKILL.md" else MAX_FILE_BYTES
        if size > limit or total > MAX_TOTAL_BYTES or len(selected) >= MAX_FILES:
            raise InstallError("GitHub skill exceeds package size limits; no package files were downloaded")
        selected.append((rel, entry["mode"], object_id(entry.get("sha")), size))
    if not any(rel == "SKILL.md" for rel, _, _, _ in selected):
        raise InstallError("No SKILL.md at the selected directory; select the skill subdirectory")

    # Only validated metadata can reach the content reader. Pin every URL to
    # the resolved commit and verify both byte size and Git blob identity.
    payload.mkdir()
    prefix = "" if source["path"] == "." else source["path"] + "/"
    for rel, mode, oid, size in selected:
        url = "https://raw.githubusercontent.com/" + source["repo"] + "/" + commit + "/" + quote(prefix + rel, safe="/")
        content = github_read(url, size, deadline)
        blob = b"blob " + str(len(content)).encode("ascii") + b"\0" + content
        if len(content) != size or hashlib.sha1(blob).hexdigest() != oid:
            raise InstallError("GitHub file differs from the pinned metadata")
        target = payload / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("xb") as handle:
            handle.write(content)
        target.chmod(0o755 if mode == "100755" else 0o644)
    return {**source, "commit": commit}


def protected_names() -> set[str]:
    # Works both in the source tree and the content-addressed bundled directory.
    root = Path(__file__).resolve().parents[2]
    return {entry.name.casefold() for entry in root.iterdir()
            if entry.is_dir() and (entry / "SKILL.md").is_file()}


def check_conflicts(root: Path, name: str) -> None:
    check_components(root)
    if name.casefold() in protected_names():
        raise InstallError("Installing over a bundled skill is not supported")
    if not root.exists():
        return
    for entry in root.iterdir():
        if entry.name.casefold() == name.casefold():
            raise InstallError("Destination already exists; existing skills are never overwritten")
        if not is_link(entry.lstat()) and entry.is_dir():
            skill = entry / "SKILL.md"
            if skill.is_file() and not skill.is_symlink() and skill.stat().st_size <= MAX_SKILL_BYTES:
                try:
                    content, _ = read_regular(skill, MAX_SKILL_BYTES)
                    existing_name = identity(content, allow_inactive=True)["name"]
                except InstallError:
                    continue
                if existing_name.casefold() == name.casefold():
                    raise InstallError("An installed skill declares the same name in another folder")


def prepare(args: argparse.Namespace) -> dict:
    if args.project:
        project = Path(args.project).expanduser().resolve(strict=True)
        if not project.is_dir():
            raise InstallError("--project must identify the session's project directory")
        root, scope = project / ".hope-agent" / "skills", "project"
    else:
        data_dir = os.environ.get("HA_DATA_DIR") or str(Path.home() / ".hope-agent")
        if not Path(data_dir).is_absolute():
            raise InstallError("HA_DATA_DIR must be absolute for installation outside the app process")
        root, scope = Path(data_dir).expanduser().resolve() / "skills", "managed"
    check_components(root)
    staging = Path(tempfile.mkdtemp(prefix="hope-skill-")).resolve()
    # Three levels below the temp root: outside Hope's two-level discovery
    # even when an owner has added a broad ancestor as an extra skills source.
    payload = staging / SNAPSHOT_PATH
    try:
        payload.parent.mkdir()
        if args.local:
            if args.ref or args.path:
                raise InstallError("--local already identifies a skill directory; omit --ref and --path")
            local = Path(args.local).expanduser()
            if is_link(local.lstat()):
                raise InstallError("Local source cannot be a symlink or reparse point")
            local = local.resolve(strict=True)
            files = inventory(local)
            copy_snapshot(local, payload, files)
            source = {"kind": "local", "path": str(local)}
        else:
            source = github_source(args.repo, args.url, args.path, args.ref)
            source = acquire_github(source, payload)
        files = inventory(payload)
        metadata = identity((payload / "SKILL.md").read_bytes())
        check_conflicts(root, metadata["name"])
        plan = {"schemaVersion": 1, "source": source, "scope": scope,
                "previewRoot": str(staging),
                "root": str(root), "target": str(root / metadata["name"]),
                **metadata, "files": files}
        plan_path = staging / "plan.json"
        plan_path.write_text(json.dumps(plan, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
        return {"status": "prepared", "plan": str(plan_path), "expectedDigest": digest(plan),
                "reviewDirectory": str(payload), **plan}
    except BaseException:
        shutil.rmtree(staging)
        raise


def publish_directory(source: Path, root: Path, name: str) -> None:
    """Atomically publish without replacing even an empty concurrent target."""
    check_components(root)
    if sys.platform == "win32":
        os.rename(source, root / name)  # Windows rename refuses existing destinations.
        return
    libc = ctypes.CDLL(None, use_errno=True)
    function_name, flag = ("renameatx_np", 4) if sys.platform == "darwin" else ("renameat2", 1)
    function = getattr(libc, function_name, None)
    if function is None:
        raise InstallError("This platform lacks atomic no-replace directory publication")
    function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    function.restype = ctypes.c_int
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    source_fd = os.open(source.parent, flags)
    try:
        target_fd = os.open(root, flags)
        try:
            result = function(source_fd, os.fsencode(source.name), target_fd, os.fsencode(name), flag)
            if result != 0:
                code = ctypes.get_errno()
                if code in {errno.EEXIST, errno.ENOTEMPTY}:
                    raise InstallError("Destination appeared during install; nothing was overwritten")
                raise InstallError("Atomic skill publication failed (OS error %d)" % code)
        finally:
            os.close(target_fd)
    finally:
        os.close(source_fd)


def read_plan(plan_path: Path, expected_digest: str) -> tuple[Path, dict]:
    try:
        content, _ = read_regular(plan_path, 512 * 1024)
    except FileNotFoundError:
        raise InstallError("Preview is missing or already cleaned up; prepare again") from None
    plan_path = plan_path.resolve(strict=True)
    plan = json.loads(content)
    if not isinstance(plan, dict):
        raise InstallError("Invalid installation plan")
    if not re.fullmatch(r"[a-f0-9]{64}", expected_digest) or digest(plan) != expected_digest:
        raise InstallError("Plan differs from the preview; prepare and review again")
    if plan.get("schemaVersion") != 1 or plan.get("scope") not in {"managed", "project"}:
        raise InstallError("Unsupported installation plan")
    return plan_path, plan


def discard(plan_path: Path, expected_digest: str) -> dict:
    """Remove only the sealed preview, never its source or installed target."""
    plan_path, plan = read_plan(plan_path, expected_digest)
    staging = plan_path.parent
    if (plan_path.name != "plan.json" or plan.get("previewRoot") != str(staging)
            or staging.parent != Path(tempfile.gettempdir()).resolve()
            or not re.fullmatch(r"hope-skill-[A-Za-z0-9_-]+", staging.name)):
        raise InstallError("Cleanup requires the original installer preview directory")
    check_components(staging)
    if {entry.name for entry in staging.iterdir()} - {"plan.json", "review"}:
        raise InstallError("Preview directory contains unrelated files; cleanup refused")
    review = staging / SNAPSHOT_PATH.parent
    check_components(review / SNAPSHOT_PATH.name)
    if review.exists():
        if {entry.name for entry in review.iterdir()} - {SNAPSHOT_PATH.name}:
            raise InstallError("Review directory contains unrelated files; cleanup refused")
        # Keep the plan until the large payload is gone so cleanup can be retried
        # after a partial filesystem failure. rmtree does not follow file links.
        shutil.rmtree(review)
    plan_path.unlink()
    staging.rmdir()
    return {"status": "discarded", "plan": str(plan_path)}


def install(plan_path: Path, expected_digest: str) -> dict:
    plan_path, plan = read_plan(plan_path, expected_digest)
    payload = plan_path.parent / SNAPSHOT_PATH
    files = inventory(payload)
    metadata = identity((payload / "SKILL.md").read_bytes())
    if files != plan["files"] or any(plan[key] != value for key, value in metadata.items()):
        raise InstallError("Package differs from the preview; prepare and review again")
    root = Path(plan["root"])
    name = metadata["name"]
    if not root.is_absolute() or plan["target"] != str(root / name):
        raise InstallError("Invalid target in installation plan")
    check_conflicts(root, name)
    root.mkdir(parents=True, exist_ok=True)
    check_components(root)
    # Stage beside the skills root, outside every directory discovery scans.
    with tempfile.TemporaryDirectory(prefix=".hope-skill-install-", dir=root.parent) as temp:
        candidate = Path(temp) / "stage" / "payload"
        candidate.parent.mkdir()
        copy_snapshot(payload, candidate, files)
        if inventory(candidate) != files:
            raise InstallError("Copied package failed verification")
        receipt = {"schemaVersion": 1, "source": plan["source"], "files": files,
                   "previewDigest": expected_digest}
        (candidate / RECEIPT).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        check_conflicts(root, name)
        publish_directory(candidate, root, name)
    result = {"status": "installed", "name": name, "target": str(root / name),
              "source": plan["source"], "fileCount": len(files), "previewDigest": expected_digest,
              "next": {"tool": "skill", "arguments": {"name": name, "action": "inspect"}}}
    try:
        discard(plan_path, expected_digest)
        result["previewCleanup"] = "removed"
    except (InstallError, OSError, ValueError, KeyError, TypeError):
        # Publication already succeeded. Do not encourage a second installation
        # merely because removing its temporary review copy failed.
        result["previewCleanup"] = "pending"
        result["cleanup"] = {"action": "discard", "plan": str(plan_path),
                             "expectedDigest": expected_digest}
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    preview = actions.add_parser("prepare", help="Acquire a snapshot and print a reviewable JSON plan")
    sources = preview.add_mutually_exclusive_group(required=True)
    sources.add_argument("--repo", help="Public GitHub owner/repository")
    sources.add_argument("--url", help="GitHub repository, tree, or SKILL.md blob URL")
    sources.add_argument("--local", help="Local skill directory (also for private repository checkouts)")
    preview.add_argument("--path", help="Skill directory relative to repository root")
    preview.add_argument("--ref", help="Branch, tag, or commit; defaults to remote HEAD")
    preview.add_argument("--project", help="Explicit project root; default installs to the managed skills directory")
    apply = actions.add_parser("install", help="Publish exactly the prepared snapshot without overwriting")
    apply.add_argument("--plan", required=True, type=Path)
    apply.add_argument("--expected-digest", required=True)
    cleanup = actions.add_parser("discard", help="Remove an abandoned preview using its original plan and digest")
    cleanup.add_argument("--plan", required=True, type=Path)
    cleanup.add_argument("--expected-digest", required=True)
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            result = prepare(args)
        elif args.action == "install":
            result = install(args.plan, args.expected_digest)
        else:
            result = discard(args.plan, args.expected_digest)
    except (InstallError, OSError, ValueError, KeyError, TypeError) as exc:
        # Foreign exceptions may embed downloaded text or credentials in paths.
        message = str(exc) if isinstance(exc, InstallError) else "Installation failed: invalid input or filesystem state"
        print(json.dumps({"status": "error", "error": message}), file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
