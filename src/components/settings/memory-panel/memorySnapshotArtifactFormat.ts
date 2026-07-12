import type { MemoryDbSnapshotRestorePreview, MemoryRepairArtifactFile } from "./types"

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B"
  const units = ["B", "KB", "MB", "GB"]
  let size = value
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`
}

export function shortSha256(value: string): string {
  const normalized = value.trim()
  if (normalized.length <= 16) return normalized
  return `${normalized.slice(0, 12)}...${normalized.slice(-4)}`
}

export interface MemorySnapshotArtifactSummaryParts {
  count: number
  name: string
  size: string
  sha: string
}

export function memorySnapshotArtifactSummaryParts(
  files: MemoryRepairArtifactFile[],
): MemorySnapshotArtifactSummaryParts | null {
  if (files.length === 0) return null
  const dbFile = files.find((file) => file.name === "memory.db") ?? files[0]
  return {
    count: files.length,
    name: dbFile.name,
    size: formatBytes(dbFile.sizeBytes),
    sha: shortSha256(dbFile.sha256),
  }
}

export function formatMemorySnapshotArtifactDiagnostics(
  artifactPath: string,
  files: MemoryRepairArtifactFile[],
): string {
  const lines = [
    "# Memory DB Safety Snapshot",
    "",
    `- Path: ${artifactPath}`,
    `- Files: ${files.length}`,
    "",
    "## File Verification",
    "",
  ]

  if (files.length === 0) {
    lines.push("- No file metadata returned by repair report.")
  } else {
    for (const file of files) {
      lines.push(`- ${file.name}: ${formatBytes(file.sizeBytes)}, sha256=${file.sha256}`)
    }
  }

  lines.push(
    "",
    "Keep all copied SQLite files together. Verify size and sha256 before using this snapshot for manual recovery.",
  )
  return lines.join("\n")
}

export function formatMemoryDbSnapshotRestorePreviewDiagnostics(
  preview: MemoryDbSnapshotRestorePreview,
): string {
  const lines = [
    "# Memory DB Snapshot Restore Preflight",
    "",
    `- Snapshot path: ${preview.snapshotPath}`,
    `- Current DB path: ${preview.currentDbPath}`,
    `- Created at: ${preview.createdAt || "-"}`,
    `- Status: ${preview.status}`,
    `- Can restore: ${preview.canRestore ? "yes" : "no"}`,
    `- SQLite quick_check: ${preview.quickCheck || "-"}`,
    `- Files checked: ${preview.files.length}`,
    "",
    "## Issues",
    "",
  ]

  if (preview.issues.length === 0) {
    lines.push("- None")
  } else {
    for (const issue of preview.issues) {
      lines.push(`- ${issue}`)
    }
  }

  lines.push("", "## File Checks", "")
  if (preview.files.length === 0) {
    lines.push("- No files were checked.")
  } else {
    for (const file of preview.files) {
      lines.push(
        [
          `- ${file.name}`,
          `status=${file.status}`,
          `expectedSize=${formatBytes(file.expectedSizeBytes)}`,
          `actualSize=${file.actualSizeBytes == null ? "-" : formatBytes(file.actualSizeBytes)}`,
          `expectedSha256=${file.expectedSha256 || "-"}`,
          `actualSha256=${file.actualSha256 || "-"}`,
        ].join(", "),
      )
    }
  }

  lines.push(
    "",
    "This is a read-only preflight report. It does not replace, move, or delete the active memory database.",
  )
  return lines.join("\n")
}
