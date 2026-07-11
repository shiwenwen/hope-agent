import { describe, expect, it } from "vitest"
import {
  formatMemoryDbSnapshotRestorePreviewDiagnostics,
  formatMemorySnapshotArtifactDiagnostics,
  memorySnapshotArtifactSummaryParts,
  shortSha256,
} from "./memorySnapshotArtifactFormat"
import type { MemoryDbSnapshotRestorePreview, MemoryRepairArtifactFile } from "./types"

const files: MemoryRepairArtifactFile[] = [
  {
    name: "memory.db",
    sizeBytes: 2048,
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  },
  {
    name: "memory.db-wal",
    sizeBytes: 1024,
    sha256: "abcdef6789abcdef0123456789abcdef0123456789abcdef0123456789abcd",
  },
]

describe("memorySnapshotArtifactFormat", () => {
  it("returns compact artifact summary parts around memory.db", () => {
    expect(memorySnapshotArtifactSummaryParts(files)).toEqual({
      count: 2,
      name: "memory.db",
      size: "2.0 KB",
      sha: "0123456789ab...cdef",
    })
  })

  it("formats a Markdown verification report", () => {
    const markdown = formatMemorySnapshotArtifactDiagnostics("/tmp/snapshot", files)

    expect(markdown).toContain("# Memory DB Safety Snapshot")
    expect(markdown).toContain("- Path: /tmp/snapshot")
    expect(markdown).toContain("- Files: 2")
    expect(markdown).toContain(
      "- memory.db: 2.0 KB, sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    expect(markdown).toContain("Verify size and sha256 before using this snapshot")
  })

  it("handles missing file metadata explicitly", () => {
    expect(memorySnapshotArtifactSummaryParts([])).toBeNull()
    expect(formatMemorySnapshotArtifactDiagnostics("/tmp/snapshot", [])).toContain(
      "- No file metadata returned by repair report.",
    )
  })

  it("does not shorten already short hashes", () => {
    expect(shortSha256("abc123")).toBe("abc123")
  })

  it("formats a restore preflight report for support diagnostics", () => {
    const preview: MemoryDbSnapshotRestorePreview = {
      snapshotPath: "/tmp/snapshot",
      currentDbPath: "/tmp/memory.db",
      createdAt: "2026-07-07T10:00:00.000Z",
      status: "ready",
      canRestore: true,
      quickCheck: "ok",
      issues: [],
      files: [
        {
          name: "memory.db",
          snapshotPath: "/tmp/snapshot/memory.db",
          targetPath: "/tmp/memory.db",
          status: "ok",
          expectedSizeBytes: 2048,
          actualSizeBytes: 2048,
          expectedSha256:
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
          actualSha256:
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        },
      ],
    }

    const markdown = formatMemoryDbSnapshotRestorePreviewDiagnostics(preview)

    expect(markdown).toContain("# Memory DB Snapshot Restore Preflight")
    expect(markdown).toContain("- Can restore: yes")
    expect(markdown).toContain("- SQLite quick_check: ok")
    expect(markdown).toContain("- None")
    expect(markdown).toContain("- memory.db, status=ok, expectedSize=2.0 KB")
    expect(markdown).toContain("does not replace, move, or delete")
  })

  it("formats blocked restore preflight issues explicitly", () => {
    const preview: MemoryDbSnapshotRestorePreview = {
      snapshotPath: "/tmp/snapshot",
      currentDbPath: "/tmp/memory.db",
      createdAt: null,
      status: "missing_files",
      canRestore: false,
      quickCheck: "not_checked",
      issues: ["missing file: memory.db"],
      files: [],
    }

    const markdown = formatMemoryDbSnapshotRestorePreviewDiagnostics(preview)

    expect(markdown).toContain("- Status: missing_files")
    expect(markdown).toContain("- Can restore: no")
    expect(markdown).toContain("- SQLite quick_check: not_checked")
    expect(markdown).toContain("- missing file: memory.db")
    expect(markdown).toContain("- No files were checked.")
  })
})
