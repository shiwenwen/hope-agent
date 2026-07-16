import { describe, expect, it } from "vitest"
import {
  DEFAULT_MAX_CHAT_ATTACHMENT_MB,
  MAX_MAX_CHAT_ATTACHMENT_MB,
  MIN_MAX_CHAT_ATTACHMENT_MB,
  MEBIBYTE_BYTES,
  maxChatAttachmentBytes,
  normalizeFilesystemConfig,
} from "./filesystemConfig"

describe("filesystemConfig", () => {
  it("defaults missing attachment limits to 20 MiB", () => {
    const config = normalizeFilesystemConfig({ allowRemoteWrites: true })
    expect(config).toEqual({
      allowRemoteWrites: true,
      maxChatAttachmentMb: DEFAULT_MAX_CHAT_ATTACHMENT_MB,
      maxWorkspaceUploadMb: 20,
      maxTextPreviewMb: 5,
      maxTextEditMb: 5,
      maxDocumentPreviewMb: 50,
      maxArtifactImportMb: 25,
    })
    expect(maxChatAttachmentBytes(config)).toBe(20 * MEBIBYTE_BYTES)
  })

  it("rounds and clamps configured limits", () => {
    expect(normalizeFilesystemConfig({ maxChatAttachmentMb: 0 }).maxChatAttachmentMb).toBe(
      MIN_MAX_CHAT_ATTACHMENT_MB,
    )
    expect(normalizeFilesystemConfig({ maxChatAttachmentMb: 20.6 }).maxChatAttachmentMb).toBe(21)
    expect(
      normalizeFilesystemConfig({ maxChatAttachmentMb: 999 }).maxChatAttachmentMb,
    ).toBe(MAX_MAX_CHAT_ATTACHMENT_MB)
  })

  it("clamps edit to the effective preview limit", () => {
    const config = normalizeFilesystemConfig({ maxTextPreviewMb: 2, maxTextEditMb: 20 })
    expect(config.maxTextPreviewMb).toBe(2)
    expect(config.maxTextEditMb).toBe(2)
  })

  it("defaults and clamps the Artifact import limit independently", () => {
    expect(normalizeFilesystemConfig({}).maxArtifactImportMb).toBe(25)
    expect(normalizeFilesystemConfig({ maxArtifactImportMb: 0 }).maxArtifactImportMb).toBe(1)
    expect(normalizeFilesystemConfig({ maxArtifactImportMb: 999 }).maxArtifactImportMb).toBe(100)
  })
})
