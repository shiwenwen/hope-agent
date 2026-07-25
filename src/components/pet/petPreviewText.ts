const MARKDOWN_LINK = /!?\[([^\]]*)\]\([^)]*\)/gu
const MARKDOWN_REFERENCE_LINK = /!?\[([^\]]*)\]\s*\[[^\]]*\]/gu

/**
 * Projects Markdown assistant output into the compact, two-line Pet preview.
 * This intentionally preserves readable content rather than rendering rich
 * blocks whose layout would make the floating bubble jump while streaming.
 */
export function petPreviewText(value: string): string {
  return (
    value
      .replace(/\r\n?/gu, "\n")
      .replace(/^\s{0,3}(?:`{3,}|~{3,})[^\n]*$/gmu, " ")
      .replace(/^\s{0,3}(?:-{3,}|_{3,}|\*{3,})\s*$/gmu, " ")
      .replace(/^\s{0,3}#{1,6}(?:\s+|$)/gmu, "")
      // Durable ready previews arrive with Markdown line breaks collapsed, so a
      // former heading can appear in the middle of the single-line payload.
      .replace(/(^|\s)#{1,6}(?=\s|$)/gu, "$1")
      .replace(/^\s{0,3}>\s?/gmu, "")
      .replace(/^\s{0,3}(?:[-+*]|\d+[.)])\s+/gmu, "")
      .replace(/^\s*\[(?: |x|X)\]\s+/gmu, "")
      .replace(MARKDOWN_LINK, "$1")
      .replace(MARKDOWN_REFERENCE_LINK, "$1")
      .replace(/<((?:https?:\/\/|mailto:)[^>]+)>/giu, "$1")
      .replace(/<\/?[a-z][^>]*>/giu, " ")
      .replace(/(`+)(.*?)\1/gsu, "$2")
      .replace(/(\*\*|__|~~)(.*?)\1/gsu, "$2")
      .replace(/(^|[\s([{])([*_])(?=\S)(.*?\S)\2(?=$|[\s)\]},.!?:;])/gmu, "$1$3")
      // Streaming can expose an opening marker before its closing pair arrives.
      .replace(/`+/gu, "")
      .replace(/\*{1,3}(?=\S)/gu, "")
      .replace(/\\([\\`*_[\]{}()#+\-.!>])/gu, "$1")
      .replace(/&(?:nbsp|#160);/giu, " ")
      .replace(/&amp;/giu, "&")
      .replace(/&lt;/giu, "<")
      .replace(/&gt;/giu, ">")
      .replace(/&quot;/giu, '"')
      .replace(/&#(?:39|x27);/giu, "'")
      .split(/\s+/u)
      .filter(Boolean)
      .join(" ")
  )
}
