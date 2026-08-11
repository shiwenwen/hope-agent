export type ComposerMentionKind =
  | "file"
  | "plan"
  | "note"
  | "skill"
  | "plugin"
  | "connector"
  | "agent"

export interface ComposerMentionBinding {
  id: string
  kind: ComposerMentionKind
  targetId: string
  displayLabel: string
  /** UI-local authority binding for first-party file selections. It is never
   * serialized into IncomingTurnWire; send preparation requires the current
   * workspace root to match so a project switch cannot retarget the file. */
  workspaceRoot?: string
  raw: string
  /** JavaScript UTF-16 offsets. Converted to UTF-8 only after final text freezes. */
  start: number
  end: number
  origin?:
    | "first_party_composer_gesture"
    | "explicit_api_binding"
    | "slash_command_ast"
    | "transport_structured_binding"
}

/** Keep file provenance only while the composer is still bound to the exact
 * workspace in which the picker created it. Other mention kinds are resolved
 * by their own backend registries and do not carry filesystem authority. */
export function filterTypedMentionsForWorkspace(
  mentions: ComposerMentionBinding[],
  workspaceRoot: string | null,
): ComposerMentionBinding[] {
  return mentions.filter(
    (mention) =>
      mention.kind !== "file" || (!!workspaceRoot && mention.workspaceRoot === workspaceRoot),
  )
}

/** Exact single-range composer edit supplied by a first-party picker or other
 * structured input action. Offsets are UTF-16 boundaries in the old/new text. */
export interface ComposerMentionTextChange {
  oldStart: number
  oldEnd: number
  newEnd: number
}

/** Bounded local discovery row returned by a registered Plugin/Connector
 * mention provider. It is picker metadata only, never an authorization or
 * executable tool handle. */
export interface MentionCapabilityCandidate {
  kind: "plugin" | "connector"
  targetId: string
  displayLabel: string
  namespace: string
  summary: string
}

export interface ParsedCapabilityMention {
  kind: "plugin" | "connector"
  targetId: string
  label: string
  raw: string
  start: number
  end: number
}

/** Parse the display token shape only. Callers must separately require an
 * exact trusted `ComposerMentionBinding` before treating a result as typed. */
export function parseCapabilityMentions(input: string): ParsedCapabilityMention[] {
  const rows: ParsedCapabilityMention[] = []
  const pattern = /\[@([^\]]+)\]\(#(plugin|connector):([^)]+)\)/g
  for (const match of input.matchAll(pattern)) {
    const start = match.index ?? 0
    const raw = match[0]
    rows.push({
      kind: match[2] as ParsedCapabilityMention["kind"],
      targetId: match[3],
      label: match[1],
      raw,
      start,
      end: start + raw.length,
    })
  }
  return rows
}

export interface IncomingTurnWire {
  promptContractVersion: 3
  mentionWireVersion: 1
  userInput: {
    inputItemId: string
    canonicalizationVersion: 1
    text: string
    digest: string
  }
  mentions: Array<{
    id: string
    kind: ComposerMentionKind
    targetId: string
    displayLabel: string
    origin:
      | "first_party_composer_gesture"
      | "explicit_api_binding"
      | "slash_command_ast"
      | "transport_structured_binding"
    sourceAnchor: {
      type: "inline"
      inputItemId: string
      canonicalTextDigest: string
      startUtf8: number
      endUtf8: number
    }
  }>
}

export function newMentionId(): string {
  return (
    globalThis.crypto?.randomUUID?.() ??
    `mention-${Date.now()}-${Math.random().toString(36).slice(2)}`
  )
}

/** Build the provenance binding returned by the backend slash parser. The
 * complete command is anchored so later edits cannot retain the activation. */
export function slashSkillMentionBinding(
  text: string,
  skillName: string,
  commandName: string,
): ComposerMentionBinding {
  return {
    id: newMentionId(),
    kind: "skill",
    targetId: skillName,
    displayLabel: commandName,
    raw: text,
    start: 0,
    end: text.length,
    origin: "slash_command_ast",
  }
}

/** Preserve unaffected typed selections across an ordinary text edit. Any
 * selection intersected by the edit loses typed provenance and becomes plain
 * text, even if the resulting characters still resemble a mention token. */
export function reconcileTypedMentions(
  previous: string,
  next: string,
  mentions: ComposerMentionBinding[],
): ComposerMentionBinding[] {
  if (previous === next) return mentions
  const sharedLimit = Math.min(previous.length, next.length)
  let commonPrefix = 0
  while (commonPrefix < sharedLimit && previous[commonPrefix] === next[commonPrefix]) {
    commonPrefix += 1
  }

  // Compute the suffix independently from the prefix. When an insertion or
  // deletion contains repeated text, prefix and suffix may overlap: both
  // `T| T -> T` and `T |T -> T` are equally-small edit explanations. Picking
  // one greedily can transfer a binding from a deleted typed T onto the
  // surviving unbound lookalike.
  let commonSuffix = 0
  while (
    commonSuffix < sharedLimit &&
    previous[previous.length - 1 - commonSuffix] === next[next.length - 1 - commonSuffix]
  ) {
    commonSuffix += 1
  }

  // A single contiguous replacement keeps `prefix + suffix` characters. If
  // the independent prefix/suffix overlap, every split in this interval is an
  // equally minimal alignment. Provenance survives only when both extremes
  // map it identically; the mapping transitions monotonically from suffix to
  // changed-range to prefix, so the extremes cover every outcome.
  const firstOptimalPrefix =
    commonPrefix + commonSuffix >= sharedLimit
      ? Math.max(0, sharedLimit - commonSuffix)
      : commonPrefix
  const lastOptimalPrefix = commonPrefix
  const delta = next.length - previous.length

  const mapForAlignment = (
    mention: ComposerMentionBinding,
    prefix: number,
  ): { start: number; end: number } | null => {
    const suffix = Math.min(commonSuffix, sharedLimit - prefix)
    const oldEnd = previous.length - suffix
    if (mention.end <= prefix) return { start: mention.start, end: mention.end }
    if (mention.start >= oldEnd) {
      return { start: mention.start + delta, end: mention.end + delta }
    }
    return null
  }

  return mentions.flatMap((mention) => {
    const first = mapForAlignment(mention, firstOptimalPrefix)
    const last = mapForAlignment(mention, lastOptimalPrefix)
    if (!first || !last || first.start !== last.start || first.end !== last.end) return []
    const updated = { ...mention, start: first.start, end: first.end }
    return next.slice(updated.start, updated.end) === updated.raw ? [updated] : []
  })
}

/** Preserve typed spans across a known first-party replacement range. Unlike
 * the string-only reconciler, this never has to guess which identical token an
 * insertion/deletion affected. Invalid descriptors fail closed by falling back
 * to the ambiguity-aware generic reconciler. */
export function reconcileTypedMentionsForChange(
  previous: string,
  next: string,
  mentions: ComposerMentionBinding[],
  change: ComposerMentionTextChange,
): ComposerMentionBinding[] {
  const { oldStart, oldEnd, newEnd } = change
  const validRange =
    Number.isInteger(oldStart) &&
    Number.isInteger(oldEnd) &&
    Number.isInteger(newEnd) &&
    oldStart >= 0 &&
    oldEnd >= oldStart &&
    oldEnd <= previous.length &&
    newEnd >= oldStart &&
    newEnd <= next.length &&
    previous.slice(0, oldStart) === next.slice(0, oldStart) &&
    previous.slice(oldEnd) === next.slice(newEnd)
  if (!validRange) return reconcileTypedMentions(previous, next, mentions)

  const delta = newEnd - oldEnd
  return mentions.flatMap((mention) => {
    const updated =
      mention.end <= oldStart
        ? mention
        : mention.start >= oldEnd
          ? { ...mention, start: mention.start + delta, end: mention.end + delta }
          : null
    if (!updated) return []
    return next.slice(updated.start, updated.end) === updated.raw ? [updated] : []
  })
}

/** Canonicalize the composer text with String#trim while preserving every
 * typed binding that remains wholly inside the retained range. Trimming is
 * two disjoint edits when both ends contain whitespace, so the generic
 * single-edit reconciler is intentionally not used here. */
export function trimTextWithTypedMentions(
  text: string,
  mentions: ComposerMentionBinding[],
): { text: string; mentions: ComposerMentionBinding[] } {
  const canonicalText = text.trim()
  if (canonicalText === text) return { text, mentions }

  const leadingChars = text.length - text.trimStart().length
  const retainedEnd = leadingChars + canonicalText.length
  const canonicalMentions = mentions.flatMap((mention) => {
    if (mention.start < leadingChars || mention.end > retainedEnd) return []
    const shifted = {
      ...mention,
      start: mention.start - leadingChars,
      end: mention.end - leadingChars,
    }
    return canonicalText.slice(shifted.start, shifted.end) === shifted.raw ? [shifted] : []
  })
  return { text: canonicalText, mentions: canonicalMentions }
}

interface TextProjection {
  text: string
  /** Maps source UTF-16 boundaries to projected UTF-16 boundaries. A missing
   * boundary means the transform coalesced it with neighboring source text. */
  sourceBoundaries: Array<number | undefined>
}

function projectTypedMentions(
  source: string,
  projection: TextProjection,
  mentions: ComposerMentionBinding[],
  retainedLength = projection.text.length,
): ComposerMentionBinding[] {
  return mentions.flatMap((mention) => {
    if (
      !Number.isInteger(mention.start) ||
      !Number.isInteger(mention.end) ||
      mention.start < 0 ||
      mention.end <= mention.start ||
      mention.end > source.length ||
      source.slice(mention.start, mention.end) !== mention.raw
    ) {
      return []
    }

    const start = projection.sourceBoundaries[mention.start]
    const end = projection.sourceBoundaries[mention.end]
    if (start === undefined || end === undefined || end <= start || end > retainedLength) {
      return []
    }

    return [
      {
        ...mention,
        raw: projection.text.slice(start, end),
        start,
        end,
      },
    ]
  })
}

function normalizeLineEndings(text: string): TextProjection {
  const sourceBoundaries = new Array<number | undefined>(text.length + 1)
  let projected = ""
  let cursor = 0
  sourceBoundaries[0] = 0

  while (cursor < text.length) {
    sourceBoundaries[cursor] = projected.length
    if (text[cursor] === "\r" && text[cursor + 1] === "\n") {
      projected += "\n"
      // A span cutting through a CRLF pair cannot be mapped unambiguously.
      sourceBoundaries[cursor + 1] = undefined
      cursor += 2
    } else {
      projected += text[cursor] === "\r" ? "\n" : text[cursor]
      cursor += 1
    }
    sourceBoundaries[cursor] = projected.length
  }

  return { text: projected, sourceBoundaries }
}

/** Build the collapsed user-message preview and carry only provenance spans
 * that survive line-ending normalization, both truncation limits, and trimEnd. */
export function buildCollapsedTextPreviewWithTypedMentions(
  text: string,
  mentions: ComposerMentionBinding[],
  maxLines: number,
  maxChars: number,
): { text: string; mentions: ComposerMentionBinding[] } {
  const normalized = normalizeLineEndings(text)
  const lineLimited = normalized.text.split("\n").slice(0, maxLines).join("\n")
  const charLimited = lineLimited.length > maxChars ? lineLimited.slice(0, maxChars) : lineLimited
  const trimmed = charLimited.trimEnd()
  if (!trimmed) return { text: "...", mentions: [] }

  return {
    text: `${trimmed}...`,
    mentions: projectTypedMentions(text, normalized, mentions, trimmed.length),
  }
}

/** Collapse whitespace for a compact single-line surface while remapping only
 * the typed bindings supplied by the source message. No mention is inferred
 * from token-shaped text in the projection. */
export function collapseWhitespaceWithTypedMentions(
  text: string,
  mentions: ComposerMentionBinding[],
): { text: string; mentions: ComposerMentionBinding[] } {
  const sourceBoundaries = new Array<number | undefined>(text.length + 1)
  let projected = ""
  let cursor = 0
  sourceBoundaries[0] = 0

  while (cursor < text.length) {
    sourceBoundaries[cursor] = projected.length
    if (/\s/.test(text[cursor])) {
      const runStart = cursor
      cursor += 1
      while (cursor < text.length && /\s/.test(text[cursor])) cursor += 1

      const shouldKeepSeparator = projected.length > 0 && cursor < text.length
      if (shouldKeepSeparator) projected += " "
      for (let boundary = runStart + 1; boundary < cursor; boundary += 1) {
        sourceBoundaries[boundary] = undefined
      }
      sourceBoundaries[cursor] = projected.length
      continue
    }

    projected += text[cursor]
    cursor += 1
    sourceBoundaries[cursor] = projected.length
  }

  const projection = { text: projected, sourceBoundaries }
  return {
    text: projected,
    mentions: projectTypedMentions(text, projection, mentions),
  }
}

/** Restore a failed send ahead of any draft entered while the request was in
 * flight. Both provenance sets survive: the newer spans are shifted by the
 * exact prefix inserted before them. */
export function mergeTypedMentionDrafts(
  restoredText: string,
  restoredMentions: ComposerMentionBinding[],
  currentText: string,
  currentMentions: ComposerMentionBinding[],
): { text: string; mentions: ComposerMentionBinding[] } {
  if (!restoredText) {
    return {
      text: currentText,
      mentions: reconcileTypedMentions(currentText, currentText, currentMentions),
    }
  }
  if (!currentText) {
    return {
      text: restoredText,
      mentions: reconcileTypedMentions(restoredText, restoredText, restoredMentions),
    }
  }

  const separator = restoredText.endsWith("\n") ? "" : "\n"
  const prefixLength = restoredText.length + separator.length
  const text = `${restoredText}${separator}${currentText}`
  const shiftedCurrent = currentMentions.map((mention) => ({
    ...mention,
    start: mention.start + prefixLength,
    end: mention.end + prefixLength,
  }))
  const mentions = [...restoredMentions, ...shiftedCurrent]
    .filter((mention) => text.slice(mention.start, mention.end) === mention.raw)
    .sort((a, b) => a.start - b.start)
  return { text, mentions }
}

export interface TypedMentionRenderLink {
  kind: "file" | "plan" | "note" | "skill" | "agent" | "plugin" | "connector"
  targetId: string
  displayLabel: string
}

/** Replace verified typed spans with per-render opaque Markdown links and
 * return the trusted lookup table separately. MarkdownLink requires an entry
 * in this table, so no string embedded in user/model/history content can
 * declare itself provenance-bearing. This changes display input only. */
export function prepareTypedMentionLinks(
  text: string,
  mentions: ComposerMentionBinding[],
  markerNamespace = newMentionId(),
): { text: string; links: Map<string, TypedMentionRenderLink> } {
  let rendered = text
  const links = new Map<string, TypedMentionRenderLink>()
  const eligible = mentions
    .filter(
      (mention): mention is ComposerMentionBinding & { kind: TypedMentionRenderLink["kind"] } =>
        mention.kind === "skill" ||
        mention.kind === "file" ||
        mention.kind === "plan" ||
        mention.kind === "note" ||
        mention.kind === "agent" ||
        mention.kind === "plugin" ||
        mention.kind === "connector",
    )
    .filter((mention) => text.slice(mention.start, mention.end) === mention.raw)
    .sort((a, b) => b.start - a.start)
  for (const [index, mention] of eligible.entries()) {
    const opaqueHref = `#ha-mention-${markerNamespace}-${index}`
    links.set(opaqueHref, {
      kind: mention.kind,
      targetId: mention.targetId,
      displayLabel: mention.displayLabel,
    })
    if (mention.kind === "file" || mention.kind === "plan" || mention.kind === "note") {
      rendered = `${rendered.slice(0, mention.start)}[${mention.kind}](${opaqueHref})${rendered.slice(mention.end)}`
      continue
    }

    const marker = `](#${mention.kind}:${mention.targetId})`
    const marked = `](${opaqueHref})`
    const raw = rendered.slice(mention.start, mention.end)
    const nextRaw = raw.replace(marker, marked)
    if (nextRaw === raw) {
      links.delete(opaqueHref)
      continue
    }
    rendered = `${rendered.slice(0, mention.start)}${nextRaw}${rendered.slice(mention.end)}`
  }
  return { text: rendered, links }
}

function utf8Offset(text: string, utf16Offset: number): number {
  return new TextEncoder().encode(text.slice(0, utf16Offset)).length
}

function rotateRight(value: number, count: number): number {
  return (value >>> count) | (value << (32 - count))
}

/** Synchronous SHA-256 fallback for the documented LAN HTTP UI, where Web
 * Crypto's SubtleCrypto is unavailable because the page is not a secure
 * context. This digest binds offsets to the submitted text; it does not
 * handle credentials or replace backend validation. */
function sha256Fallback(bytes: Uint8Array): Uint8Array {
  const constants = new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ])
  const state = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ])
  const paddedLength = Math.ceil((bytes.length + 9) / 64) * 64
  const padded = new Uint8Array(paddedLength)
  padded.set(bytes)
  padded[bytes.length] = 0x80
  const view = new DataView(padded.buffer)
  const bitLength = bytes.length * 8
  view.setUint32(paddedLength - 8, Math.floor(bitLength / 0x1_0000_0000), false)
  view.setUint32(paddedLength - 4, bitLength >>> 0, false)

  const words = new Uint32Array(64)
  for (let offset = 0; offset < paddedLength; offset += 64) {
    for (let i = 0; i < 16; i += 1) words[i] = view.getUint32(offset + i * 4, false)
    for (let i = 16; i < 64; i += 1) {
      const x = words[i - 15]
      const y = words[i - 2]
      const sigma0 = rotateRight(x, 7) ^ rotateRight(x, 18) ^ (x >>> 3)
      const sigma1 = rotateRight(y, 17) ^ rotateRight(y, 19) ^ (y >>> 10)
      words[i] = (words[i - 16] + sigma0 + words[i - 7] + sigma1) >>> 0
    }

    let [a, b, c, d, e, f, g, h] = state
    for (let i = 0; i < 64; i += 1) {
      const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25)
      const choice = (e & f) ^ (~e & g)
      const temp1 = (h + sum1 + choice + constants[i] + words[i]) >>> 0
      const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22)
      const majority = (a & b) ^ (a & c) ^ (b & c)
      const temp2 = (sum0 + majority) >>> 0
      h = g
      g = f
      f = e
      e = (d + temp1) >>> 0
      d = c
      c = b
      b = a
      a = (temp1 + temp2) >>> 0
    }
    state[0] = (state[0] + a) >>> 0
    state[1] = (state[1] + b) >>> 0
    state[2] = (state[2] + c) >>> 0
    state[3] = (state[3] + d) >>> 0
    state[4] = (state[4] + e) >>> 0
    state[5] = (state[5] + f) >>> 0
    state[6] = (state[6] + g) >>> 0
    state[7] = (state[7] + h) >>> 0
  }

  const digest = new Uint8Array(32)
  const digestView = new DataView(digest.buffer)
  state.forEach((value, index) => digestView.setUint32(index * 4, value, false))
  return digest
}

export async function sha256Digest(
  text: string,
  subtle: SubtleCrypto | null | undefined = globalThis.crypto?.subtle,
): Promise<string> {
  const bytes = new TextEncoder().encode(text)
  const digest = subtle
    ? new Uint8Array(await subtle.digest("SHA-256", bytes))
    : sha256Fallback(bytes)
  const hex = Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("")
  return `sha256:${hex}`
}

export async function buildIncomingTurnWire(
  text: string,
  mentions: ComposerMentionBinding[],
): Promise<IncomingTurnWire> {
  const valid = mentions
    .filter(
      (mention) =>
        mention.start >= 0 &&
        mention.end > mention.start &&
        mention.end <= text.length &&
        text.slice(mention.start, mention.end) === mention.raw,
    )
    .sort((a, b) => a.start - b.start)
  if (valid.some((mention, index) => index > 0 && valid[index - 1].end > mention.start)) {
    throw new Error("Typed mention ranges overlap")
  }
  const inputItemId =
    globalThis.crypto?.randomUUID?.() ??
    `input-${Date.now()}-${Math.random().toString(36).slice(2)}`
  const digest = await sha256Digest(text)
  return {
    promptContractVersion: 3,
    mentionWireVersion: 1,
    userInput: {
      inputItemId,
      canonicalizationVersion: 1,
      text,
      digest,
    },
    mentions: valid.map((mention) => ({
      id: mention.id,
      kind: mention.kind,
      targetId: mention.targetId,
      displayLabel: mention.displayLabel,
      origin: mention.origin ?? "first_party_composer_gesture",
      sourceAnchor: {
        type: "inline",
        inputItemId,
        canonicalTextDigest: digest,
        startUtf8: utf8Offset(text, mention.start),
        endUtf8: utf8Offset(text, mention.end),
      },
    })),
  }
}
