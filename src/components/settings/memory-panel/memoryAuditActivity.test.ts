import { describe, expect, test } from "vitest"
import {
  countMemoryAuditActivity,
  includeCrossSourceAudit,
  mergeMemoryAuditActivity,
  splitMemoryAuditPage,
} from "./memoryAuditActivity"

interface TestActivity {
  kind: "memory" | "experience" | "decision"
  id: string
  createdAt: string
}

function item(kind: TestActivity["kind"], id: string, createdAt: string): TestActivity {
  return { kind, id, createdAt }
}

describe("memoryAuditActivity", () => {
  test("includes cross-source activity only for All changes", () => {
    expect(includeCrossSourceAudit("all")).toBe(true)
    expect(includeCrossSourceAudit(undefined)).toBe(true)
    expect(includeCrossSourceAudit("update")).toBe(false)
    expect(includeCrossSourceAudit("delete")).toBe(false)
  })

  test("sorts legacy memory, workflow, and claim decisions together for All changes", () => {
    const merged = mergeMemoryAuditActivity({
      action: "all",
      memory: [item("memory", "m1", "2026-07-07T10:00:00Z")],
      experience: [item("experience", "w1", "2026-07-07T12:00:00Z")],
      decisions: [item("decision", "d1", "2026-07-07T11:00:00Z")],
    })

    expect(merged.map((entry) => `${entry.kind}:${entry.id}`)).toEqual([
      "experience:w1",
      "decision:d1",
      "memory:m1",
    ])
  })

  test("uses deterministic tie breakers for same-timestamp activity", () => {
    const createdAt = "2026-07-07T10:00:00Z"
    const merged = mergeMemoryAuditActivity({
      action: "all",
      memory: [item("memory", "m2", createdAt), item("memory", "m1", createdAt)],
      experience: [item("experience", "w2", createdAt), item("experience", "w1", createdAt)],
      decisions: [item("decision", "d2", createdAt), item("decision", "d1", createdAt)],
    })

    expect(merged.map((entry) => `${entry.kind}:${entry.id}`)).toEqual([
      "decision:d1",
      "decision:d2",
      "experience:w1",
      "experience:w2",
      "memory:m1",
      "memory:m2",
    ])
  })

  test("keeps action-filtered audit views legacy-only", () => {
    const merged = mergeMemoryAuditActivity({
      action: "update",
      memory: [item("memory", "m1", "2026-07-07T10:00:00Z")],
      experience: [item("experience", "w1", "2026-07-07T12:00:00Z")],
      decisions: [item("decision", "d1", "2026-07-07T11:00:00Z")],
    })

    expect(merged).toEqual([item("memory", "m1", "2026-07-07T10:00:00Z")])
  })

  test("counts loaded activity with the same cross-source boundary as merge", () => {
    expect(
      countMemoryAuditActivity({
        action: "all",
        memoryCount: 2,
        experienceCount: 3,
        decisionCount: 4,
      }),
    ).toBe(9)

    expect(
      countMemoryAuditActivity({
        action: "delete",
        memoryCount: 2,
        experienceCount: 3,
        decisionCount: 4,
      }),
    ).toBe(2)
  })

  test("splits unified audit page records back into source buckets", () => {
    const buckets = splitMemoryAuditPage({
      items: [
        { item: { kind: "claimDecision", record: item("decision", "d1", "2026-07-07T12:00:00Z") } },
        { item: { kind: "experience", record: item("experience", "w1", "2026-07-07T11:00:00Z") } },
        { item: { kind: "legacyMemory", record: item("memory", "m1", "2026-07-07T10:00:00Z") } },
      ],
      mapDecision: (decision) => ({ ...decision, id: `mapped-${decision.id}` }),
    })

    expect(buckets.memory.map((entry) => entry.id)).toEqual(["m1"])
    expect(buckets.experience.map((entry) => entry.id)).toEqual(["w1"])
    expect(buckets.decisions.map((entry) => entry.id)).toEqual(["mapped-d1"])
  })
})
