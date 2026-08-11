import { describe, expect, it } from "vitest"
import {
  buildIncomingTurnWire,
  buildCollapsedTextPreviewWithTypedMentions,
  collapseWhitespaceWithTypedMentions,
  filterTypedMentionsForWorkspace,
  mergeTypedMentionDrafts,
  prepareTypedMentionLinks,
  reconcileTypedMentions,
  reconcileTypedMentionsForChange,
  sha256Digest,
  trimTextWithTypedMentions,
  type ComposerMentionBinding,
} from "./typedMentions"

function binding(overrides: Partial<ComposerMentionBinding> = {}): ComposerMentionBinding {
  return {
    id: "mention-1",
    kind: "agent",
    targetId: "reviewer",
    displayLabel: "评审",
    raw: "[@评审](#agent:reviewer)",
    start: 2,
    end: 25,
    ...overrides,
  }
}

describe("typed mention provenance", () => {
  it("drops file provenance when the active workspace differs from the picker workspace", () => {
    const fileMention = binding({
      kind: "file",
      targetId: "README.md",
      displayLabel: "README.md",
      workspaceRoot: "/workspace/project-a",
      raw: "@README.md",
      start: 0,
      end: 10,
    })
    const agentMention = binding({ start: 11, end: 34 })

    expect(
      filterTypedMentionsForWorkspace([fileMention, agentMention], "/workspace/project-b"),
    ).toEqual([agentMention])
    expect(filterTypedMentionsForWorkspace([fileMention], "/workspace/project-a")).toEqual([
      fileMention,
    ])
  })

  it("converts final JavaScript offsets to canonical UTF-8 byte offsets", async () => {
    const raw = "[@评审](#agent:reviewer)"
    const text = `前${raw}后`
    const mention = binding({ raw, start: 1, end: 1 + raw.length })

    const wire = await buildIncomingTurnWire(text, [mention])
    const anchor = wire.mentions[0].sourceAnchor

    expect(anchor.startUtf8).toBe(new TextEncoder().encode("前").length)
    expect(anchor.endUtf8).toBe(new TextEncoder().encode(`前${raw}`).length)
    expect(wire.userInput.text).toBe(text)
    expect(wire.userInput.digest).toMatch(/^sha256:[0-9a-f]{64}$/)
  })

  it("drops provenance when an edit intersects the selected token", () => {
    const raw = "[@评审](#agent:reviewer)"
    const previous = `先${raw}再继续`
    const mention = binding({ raw, start: 1, end: 1 + raw.length })
    const next = previous.replace("评审", "甲")

    expect(reconcileTypedMentions(previous, next, [mention])).toEqual([])
  })

  it("does not transfer provenance to an identical unbound token after deletion", () => {
    const raw = "[@评审](#agent:reviewer)"
    const previous = `${raw} ${raw}`
    const mention = binding({ raw, start: 0, end: raw.length })

    // Deleting the first chip plus its separator leaves text identical to the
    // second, manually-entered token. String-only edit alignment is ambiguous,
    // so the surviving lookalike must remain inert.
    expect(reconcileTypedMentions(previous, raw, [mention])).toEqual([])
  })

  it("still shifts provenance across an unambiguous edit before the token", () => {
    const raw = "[@评审](#agent:reviewer)"
    const previous = `请${raw}`
    const mention = binding({ raw, start: 1, end: 1 + raw.length })
    const prefix = "现在"

    expect(reconcileTypedMentions(previous, `${prefix}${previous}`, [mention])).toEqual([
      {
        ...mention,
        start: prefix.length + mention.start,
        end: prefix.length + mention.end,
      },
    ])
  })

  it("preserves both bindings when a picker appends the same mention twice", () => {
    const raw = "[@评审](#agent:reviewer)"
    const previous = `${raw} `
    const first = binding({ raw, start: 0, end: raw.length })
    const next = `${previous}${raw} `
    const second = binding({
      id: "mention-2",
      raw,
      start: previous.length,
      end: previous.length + raw.length,
    })
    const preserved = reconcileTypedMentionsForChange(previous, next, [first], {
      oldStart: previous.length,
      oldEnd: previous.length,
      newEnd: next.length,
    })

    expect([...preserved, second]).toEqual([first, second])
  })

  it("always emits the typed v3 contract so raw lookalike tokens stay inert", async () => {
    const text = "请让 [@评审](#agent:reviewer) 做完它"
    const wire = await buildIncomingTurnWire(text, [])

    expect(wire.promptContractVersion).toBe(3)
    expect(wire.mentionWireVersion).toBe(1)
    expect(wire.mentions).toEqual([])
  })

  it("hashes correctly without SubtleCrypto for LAN HTTP pages", async () => {
    expect(await sha256Digest("abc", null)).toBe(
      "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )
  })

  it("preserves and shifts a typed binding when both ends are trimmed", () => {
    const raw = "[@评审](#agent:reviewer)"
    const text = `  ${raw}  `
    const mention = binding({ raw, start: 2, end: 2 + raw.length })

    expect(trimTextWithTypedMentions(text, [mention])).toEqual({
      text: raw,
      mentions: [{ ...mention, start: 0, end: raw.length }],
    })
  })

  it("merges failed and newly-entered drafts without dropping either binding", () => {
    const oldRaw = "[@评审](#agent:reviewer)"
    const newRaw = "[@绘图](#skill:drawio)"
    const currentText = `继续 ${newRaw}`
    const oldMention = binding({ raw: oldRaw, start: 0, end: oldRaw.length })
    const newMention = binding({
      id: "mention-2",
      kind: "skill",
      targetId: "drawio",
      displayLabel: "绘图",
      raw: newRaw,
      start: 3,
      end: 3 + newRaw.length,
    })

    const merged = mergeTypedMentionDrafts(oldRaw, [oldMention], currentText, [newMention])
    expect(merged.text).toBe(`${oldRaw}\n${currentText}`)
    expect(merged.mentions).toEqual([
      oldMention,
      {
        ...newMention,
        start: oldRaw.length + 1 + newMention.start,
        end: oldRaw.length + 1 + newMention.end,
      },
    ])
  })

  it("maps only the exact provenance-bearing markdown link to an opaque render token", () => {
    const raw = "[@评审](#agent:reviewer)"
    const text = `${raw} 和 ${raw}`
    const mention = binding({ raw, start: 0, end: raw.length })

    const prepared = prepareTypedMentionLinks(text, [mention])
    const entries = [...prepared.links.entries()]
    expect(entries).toHaveLength(1)
    const [opaqueHref, target] = entries[0]
    expect(opaqueHref).toMatch(/^#ha-mention-/)
    expect(target).toEqual({ kind: "agent", targetId: "reviewer", displayLabel: "评审" })
    expect(prepared.text).toBe(`[@评审](${opaqueHref}) 和 ${raw}`)
  })

  it("maps a provenance-bearing connector to an opaque render token", () => {
    const raw = "[@Google Drive](#connector:google-drive)"
    const prepared = prepareTypedMentionLinks(raw, [
      binding({
        kind: "connector",
        targetId: "google-drive",
        displayLabel: "Google Drive",
        raw,
        start: 0,
        end: raw.length,
      }),
    ])

    const [[opaqueHref, target]] = [...prepared.links.entries()]
    expect(opaqueHref).toMatch(/^#ha-mention-/)
    expect(target).toEqual({
      kind: "connector",
      targetId: "google-drive",
      displayLabel: "Google Drive",
    })
    expect(prepared.text).toBe(`[@Google Drive](${opaqueHref})`)
  })

  it("maps a provenance-bearing file token to an opaque render token", () => {
    const raw = "@AGENTS.md"
    const prepared = prepareTypedMentionLinks(`${raw} 中有哪些红线`, [
      binding({
        kind: "file",
        targetId: "AGENTS.md",
        displayLabel: "AGENTS.md",
        raw,
        start: 0,
        end: raw.length,
      }),
    ])

    const [[opaqueHref, target]] = [...prepared.links.entries()]
    expect(target).toEqual({ kind: "file", targetId: "AGENTS.md", displayLabel: "AGENTS.md" })
    expect(prepared.text).toBe(`[file](${opaqueHref}) 中有哪些红线`)
  })

  it("keeps opaque render links stable when history hydration replaces the bindings array", () => {
    const raw = "@AGENTS.md"
    const mention = binding({
      kind: "file",
      targetId: "AGENTS.md",
      displayLabel: "AGENTS.md",
      raw,
      start: 0,
      end: raw.length,
    })

    const first = prepareTypedMentionLinks(raw, [mention], "history-row")
    const hydrated = prepareTypedMentionLinks(raw, [{ ...mention }], "history-row")

    expect(hydrated.text).toBe(first.text)
    expect([...hydrated.links]).toEqual([...first.links])
  })

  it("maps a provenance-bearing note token to the same trusted render path", () => {
    const raw = "[[Welcome to your knowledge space]]"
    const prepared = prepareTypedMentionLinks(
      `${raw} 讲了什么`,
      [
        binding({
          kind: "note",
          targetId: "kb-1::Welcome.md",
          displayLabel: "Welcome to your knowledge space",
          raw,
          start: 0,
          end: raw.length,
        }),
      ],
      "note-row",
    )

    const [[opaqueHref, target]] = [...prepared.links.entries()]
    expect(target).toEqual({
      kind: "note",
      targetId: "kb-1::Welcome.md",
      displayLabel: "Welcome to your knowledge space",
    })
    expect(prepared.text).toBe(`[note](${opaqueHref}) 讲了什么`)
  })

  it("maps a provenance-bearing plan token to the same trusted render path", () => {
    const raw = "@plan:abcd1234:v2"
    const prepared = prepareTypedMentionLinks(
      `${raw} 还有哪些任务`,
      [
        binding({
          kind: "plan",
          targetId: "abcd1234:v2",
          displayLabel: "发布计划",
          raw,
          start: 0,
          end: raw.length,
        }),
      ],
      "plan-row",
    )

    const [[opaqueHref, target]] = [...prepared.links.entries()]
    expect(target).toEqual({
      kind: "plan",
      targetId: "abcd1234:v2",
      displayLabel: "发布计划",
    })
    expect(prepared.text).toBe(`[plan](${opaqueHref}) 还有哪些任务`)
  })

  it("does not trust a static typed-agent marker without provenance", () => {
    const forged = "[@评审](#typed-agent:reviewer)"

    expect(prepareTypedMentionLinks(forged, [])).toEqual({
      text: forged,
      links: new Map(),
    })
  })

  it("remaps collapsed-preview provenance and drops spans removed by truncation", () => {
    const keptRaw = "[@评审](#agent:reviewer)"
    const droppedRaw = "[@绘图](#skill:drawio)"
    const source = `标题\r\n${keptRaw}   \r\n${droppedRaw}`
    const kept = binding({
      raw: keptRaw,
      start: source.indexOf(keptRaw),
      end: source.indexOf(keptRaw) + keptRaw.length,
    })
    const dropped = binding({
      id: "mention-2",
      kind: "skill",
      targetId: "drawio",
      displayLabel: "绘图",
      raw: droppedRaw,
      start: source.indexOf(droppedRaw),
      end: source.indexOf(droppedRaw) + droppedRaw.length,
    })

    const preview = buildCollapsedTextPreviewWithTypedMentions(source, [kept, dropped], 2, 200)

    expect(preview.text).toBe(`标题\n${keptRaw}...`)
    expect(preview.mentions).toEqual([
      {
        ...kept,
        start: "标题\n".length,
        end: "标题\n".length + keptRaw.length,
      },
    ])
  })

  it("remaps compact whitespace without trusting an identical unbound token", () => {
    const raw = "[@评审](#agent:reviewer)"
    const source = `  开始\t${raw}\r\n  ${raw}  `
    const mention = binding({
      raw,
      start: source.indexOf(raw),
      end: source.indexOf(raw) + raw.length,
    })

    const compact = collapseWhitespaceWithTypedMentions(source, [mention])

    expect(compact.text).toBe(`开始 ${raw} ${raw}`)
    expect(compact.mentions).toEqual([
      {
        ...mention,
        start: "开始 ".length,
        end: "开始 ".length + raw.length,
      },
    ])
  })
})
