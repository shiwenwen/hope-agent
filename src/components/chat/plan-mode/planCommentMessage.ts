type TranslationFn = (key: string, options?: Record<string, unknown>) => unknown

export interface PlanCommentPayload {
  selectedText: string
  comment: string
}

export interface BuiltPlanComment {
  /** Full XML payload sent to the LLM (it needs the unambiguous
   *  `<selected-text>` / `<revision-request>` structure to know which section
   *  to revise and resubmit via submit_plan). */
  prompt: string
  /** Human-readable markdown stored in the message DB. Used by IM channels
   *  that have no React UI — the layered "header / quote / comment" form
   *  reads naturally as plain text in Telegram, Slack, etc. The desktop GUI
   *  ignores this and renders {@link PlanCommentBubble} from `payload`
   *  instead, so this string is purely a fallback. */
  displayText: string
  /** Structured payload routed through `attachments_meta.plan_comment` so the
   *  desktop GUI can render a custom bubble instead of the markdown above. */
  payload: PlanCommentPayload
}

export function buildPlanCommentMessage(
  selectedText: string,
  comment: string,
  t: TranslationFn,
): BuiltPlanComment {
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

  // IM-friendly 3-segment markdown: header chip line → quoted selection
  // → user's comment. Quoted section preserves multi-line input via per-line
  // `> ` prefix so it renders as a single blockquote in any standard
  // markdown renderer (Telegram / Slack / WeChat all handle this).
  // IM markdown explicitly prepends 💬 — the i18n key holds plain text only
  // so the desktop GUI can pair the same string with a lucide icon component
  // for a more polished render.
  const header = String(t("planMode.commentDisplay"))
  const quoteLines = selectedText
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n")
  const displayText = `💬 ${header}\n\n${quoteLines}\n\n${comment}`

  return {
    prompt,
    displayText,
    payload: { selectedText, comment },
  }
}
