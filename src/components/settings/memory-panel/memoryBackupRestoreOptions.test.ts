import { describe, expect, it } from "vitest"

import { buildMemoryBackupStructuredRestoreOptions } from "./types"

describe("memory backup restore options", () => {
  it("explicitly enables every structured memory asset class by default", () => {
    expect(buildMemoryBackupStructuredRestoreOptions(false)).toEqual({
      restoreClaims: true,
      restoreProfileSnapshots: true,
      restoreEpisodes: true,
      restoreProcedures: true,
      restoreExperienceHistory: true,
      allowProfileScopeConflicts: false,
    })
  })

  it("keeps profile conflict replacement as the only user-controlled override", () => {
    expect(buildMemoryBackupStructuredRestoreOptions(true)).toMatchObject({
      restoreClaims: true,
      restoreProfileSnapshots: true,
      restoreEpisodes: true,
      restoreProcedures: true,
      restoreExperienceHistory: true,
      allowProfileScopeConflicts: true,
    })
  })
})
