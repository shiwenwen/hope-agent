type TranslationFn = (key: string, options?: Record<string, unknown>) => unknown

/** Build the prompt + display pair for an inline plan comment.
 *
 * - `prompt` is the full XML payload sent to the LLM (it needs the unambiguous
 *   `<selected-text>` / `<revision-request>` structure so it knows which
 *   section to revise and resubmit via submit_plan).
 * - `displayText` is the human-readable version persisted to the message DB
 *   and rendered in the user bubble — quoted selection on top, comment below.
 *   Without this split, the user sees the raw XML in their own bubble, which
 *   reads like garbage and breaks the chat flow. */
export function buildPlanCommentMessage(
  selectedText: string,
  comment: string,
  t: TranslationFn,
): { prompt: string; displayText: string } {
  const prompt = [
    `<plan-inline-comment>`,
    `The user selected the following section from the current plan and requests a revision:`,
    ``,
    `<selected-text>`,
    selectedText,
    `</selected-text>`,
    ``,
    `<revision-request>`,
    comment,
    `</revision-request>`,
    ``,
    `Please revise the plan to address this feedback. Modify the quoted section while keeping the rest of the plan intact, then resubmit the updated plan using the submit_plan tool.`,
    `</plan-inline-comment>`,
  ].join("\n")

  // Layout: comment up top as the main message, selection below as a quoted
  // footnote — mirrors reply patterns in Slack / Discord where the user's
  // own words are the focal point and the quoted context sits underneath.
  // The 💬 + label is inlined into the blockquote's first line so the whole
  // footnote is one visual unit instead of a separate header row.
  const quoteLabel = String(t("planMode.commentQuotedFrom"))
  const selectionLines = selectedText.split("\n")
  const firstLine = selectionLines[0] ?? ""
  const restLines = selectionLines.slice(1).map((line) => `> ${line}`)
  const quoteBlock = [`> 💬 ${quoteLabel} · ${firstLine}`, ...restLines].join("\n")
  const displayText = `${comment}\n\n${quoteBlock}`

  return { prompt, displayText }
}
