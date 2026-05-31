export const TOOL_JOB_AGENT_PREFIX = "tool_job:"
export const TOOL_JOB_STATUSES = new Set([
  "completed",
  "failed",
  "timed_out",
  "cancelled",
  "interrupted",
  "running",
])

function getXmlishAttribute(attrs: string, name: string): string | undefined {
  const match = attrs.match(new RegExp(`\\b${name}="([^"]*)"`))
  return match?.[1]
}

function getXmlishElement(content: string, name: string): string | undefined {
  const match = content.match(new RegExp(`<${name}>([\\s\\S]*?)</${name}>`))
  return match?.[1]?.trim()
}

function decodeXmlishText(value: string | undefined): string | undefined {
  if (!value) return undefined
  return value
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
}

function hasSubagentResultEnvelope(content: string): boolean {
  const trimmed = content.trimStart()
  return /^<subagent-result(?:\s|>)/.test(trimmed) && trimmed.includes("</subagent-result>")
}

export function parseToolJobPayload(
  content: string,
): { toolName?: string; status?: string; detail?: string } | null {
  const match = content.match(/<tool-job-result\b([^>]*)>/)
  if (match) {
    const attrs = match[1] || ""
    return {
      toolName: decodeXmlishText(getXmlishAttribute(attrs, "tool")),
      status: decodeXmlishText(getXmlishAttribute(attrs, "status")),
      detail:
        decodeXmlishText(getXmlishElement(content, "output")) ||
        decodeXmlishText(getXmlishElement(content, "error")) ||
        decodeXmlishText(getXmlishElement(content, "note")),
    }
  }

  if (!content.includes("<task-notification>")) {
    return null
  }
  return {
    toolName: decodeXmlishText(getXmlishElement(content, "tool")),
    status: decodeXmlishText(getXmlishElement(content, "status")),
    detail:
      decodeXmlishText(getXmlishElement(content, "output-preview")) ||
      decodeXmlishText(getXmlishElement(content, "error")) ||
      decodeXmlishText(getXmlishElement(content, "summary")) ||
      decodeXmlishText(getXmlishElement(content, "output-file")),
  }
}

export function parseSubagentResultDetail(content: string): string | undefined {
  if (hasSubagentResultEnvelope(content)) {
    return (
      decodeXmlishText(getXmlishElement(content, "result")) ||
      decodeXmlishText(getXmlishElement(content, "error"))
    )
  }

  const match = content.match(
    /<<<BEGIN_SUBAGENT_RESULT>>>\n?([\s\S]*?)\n?<<<END_SUBAGENT_RESULT>>>/,
  )
  return match?.[1]?.trim()
}

export function parseSubagentResultStatus(content: string): string {
  const status = hasSubagentResultEnvelope(content)
    ? decodeXmlishText(getXmlishElement(content, "status"))
    : content.match(/^Status:\s*(\S+)/m)?.[1]
  switch (status) {
    case "completed":
      return "completed"
    case "timeout":
    case "timed_out":
      return "timed_out"
    case "killed":
      return "cancelled"
    case "running":
    case "spawning":
      return "running"
    case "error":
      return "failed"
    default:
      return "completed"
  }
}
