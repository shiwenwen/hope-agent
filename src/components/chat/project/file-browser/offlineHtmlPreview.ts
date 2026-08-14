import type { DefaultTreeAdapterTypes } from "parse5"

const OFFLINE_HTML_PREVIEW_CSP =
  "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'none'; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'"

const REMOVED_ELEMENTS = new Set([
  "base",
  "embed",
  "frame",
  "frameset",
  "iframe",
  "object",
  "portal",
  "script",
])

const REMOVED_ATTRIBUTES = new Set([
  "action",
  "download",
  "formaction",
  "formtarget",
  "manifest",
  "ping",
  "srcdoc",
  "srcset",
  "target",
])

const OFFLINE_URL_ATTRIBUTES = new Set([
  "background",
  "href",
  "longdesc",
  "poster",
  "src",
  "usemap",
])

type ParentNode = DefaultTreeAdapterTypes.ParentNode
type ChildNode = DefaultTreeAdapterTypes.ChildNode
type Element = DefaultTreeAdapterTypes.Element

function isElement(node: ChildNode): node is Element {
  return "tagName" in node
}

function isOfflineUrl(value: string): boolean {
  const normalized = value.trim().toLowerCase()
  return (
    normalized.startsWith("#") || normalized.startsWith("blob:") || normalized.startsWith("data:")
  )
}

function sanitizeAttributes(element: Element): void {
  const tagName = element.tagName.toLowerCase()
  element.attrs = element.attrs.filter((attribute) => {
    const name = attribute.name.toLowerCase()
    if (name.startsWith("on") || REMOVED_ATTRIBUTES.has(name)) return false

    if (OFFLINE_URL_ATTRIBUTES.has(name)) {
      // Anchors and image-map areas can navigate the preview even when an
      // iframe has an empty sandbox. Keep their text/shape but remove href.
      if (name === "href" && (tagName === "a" || tagName === "area")) return false
      return isOfflineUrl(attribute.value)
    }

    return true
  })
}

function sanitizeChildren(parent: ParentNode): void {
  const retained: ChildNode[] = []

  for (const child of parent.childNodes) {
    if (!isElement(child)) {
      retained.push(child)
      continue
    }

    const tagName = child.tagName.toLowerCase()
    const isHttpEquivMeta =
      tagName === "meta" &&
      child.attrs.some((attribute) => attribute.name.toLowerCase() === "http-equiv")
    if (REMOVED_ELEMENTS.has(tagName) || isHttpEquivMeta) {
      child.parentNode = null
      continue
    }

    sanitizeAttributes(child)
    sanitizeChildren(child)
    if (tagName === "template" && "content" in child) {
      sanitizeChildren(child.content)
    }
    retained.push(child)
  }

  parent.childNodes = retained
}

function findElement(parent: ParentNode, tagName: string): Element | null {
  for (const child of parent.childNodes) {
    if (!isElement(child)) continue
    if (child.tagName.toLowerCase() === tagName) return child
    const nested = findElement(child, tagName)
    if (nested) return nested
  }
  return null
}

/**
 * Build a static, offline document for an ordinary HTML file preview.
 *
 * Parsing happens only after the user switches to rendered mode, so parse5 is
 * kept out of the initial file-browser chunk. The sanitizer removes every
 * script/navigation primitive before serialization; CSP and the iframe's
 * empty sandbox remain independent execution/network boundaries.
 */
export async function buildOfflineHtmlPreview(source: string): Promise<string> {
  const { defaultTreeAdapter, html, parse, serialize } = await import("parse5")
  // The iframe has scripting disabled, so parse noscript contents as markup as
  // the browser will; otherwise a refresh meta nested there could evade the walk.
  const document = parse(source, { scriptingEnabled: false })
  sanitizeChildren(document)

  const head = findElement(document, "head")
  if (!head) throw new Error("HTML preview has no document head")

  const policy = defaultTreeAdapter.createElement("meta", html.NS.HTML, [
    { name: "http-equiv", value: "Content-Security-Policy" },
    { name: "content", value: OFFLINE_HTML_PREVIEW_CSP },
  ])
  const firstChild = head.childNodes[0]
  if (firstChild) {
    defaultTreeAdapter.insertBefore(head, policy, firstChild)
  } else {
    defaultTreeAdapter.appendChild(head, policy)
  }

  return serialize(document)
}
