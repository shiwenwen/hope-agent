import { describe, expect, test } from "vitest"
import { petImportFailureDiagnostic } from "./petImportDiagnostics"

describe("petImportFailureDiagnostic", () => {
  test("keeps the safe missing-field evidence from a Tauri argument rejection", () => {
    expect(
      petImportFailureDiagnostic(
        { kind: "candidate", candidateId: "candidate-secret" },
        "invalid args `request` for command `pet_import_preview_cmd`: missing field `candidate_id`",
      ),
    ).toEqual({
      command: "pet_import_preview_cmd",
      sourceKind: "candidate",
      errorKind: "invalid_args",
      field: "candidate_id",
    })
  })

  test("never persists paths, URLs, upload ids, or raw invalid values", () => {
    const diagnostic = petImportFailureDiagnostic(
      { kind: "link", link: "https://example.test/pet.png?token=secret" },
      'invalid args `request`: invalid value "/Users/alice/private/pet.json?token=secret"',
    )
    const serialized = JSON.stringify(diagnostic)
    expect(diagnostic).toEqual({
      command: "pet_import_preview_cmd",
      sourceKind: "link",
      errorKind: "invalid_args",
    })
    expect(serialized).not.toContain("secret")
    expect(serialized).not.toContain("/Users/")
    expect(serialized).not.toContain("example.test")
  })

  test("extracts bounded Pet error codes without logging the raw error", () => {
    expect(
      petImportFailureDiagnostic(
        { kind: "uploadedPath", uploadId: "upload-secret" },
        "pet_zip_invalid: /private/archive.zip",
      ),
    ).toEqual({
      command: "pet_import_preview_cmd",
      sourceKind: "uploadedPath",
      errorKind: "pet_zip_invalid",
    })
  })
})
