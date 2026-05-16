import {
  useState,
  useEffect,
  useLayoutEffect,
  useRef,
  useMemo,
  type AnchorHTMLAttributes,
} from "react"
import {
  Streamdown,
  createAnimatePlugin,
  defaultRehypePlugins,
  type AnimateOptions,
  type PluginConfig,
} from "streamdown"

type AnimatePlugin = ReturnType<typeof createAnimatePlugin>
import { code } from "@streamdown/code"
import { cjk } from "@streamdown/cjk"
import {
  File as FileIcon,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  Globe,
  Hash,
  Link2,
  Mail,
  type LucideIcon,
} from "lucide-react"
import "streamdown/styles.css"
import i18next from "i18next"
import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { cn } from "@/lib/utils"
import { findAutoLinkMatches } from "@/lib/autoLink"

// Math and mermaid plugins are lazy-loaded on first use to reduce initial bundle size.
// KaTeX (~300KB) and Mermaid (~200KB) are only loaded when content requires them.
let cachedMath: PluginConfig["math"] | null = null
let cachedMermaid: PluginConfig["mermaid"] | null = null
let mathLoading = false
let mermaidLoading = false

const HAS_MATH = /\$\$|\\[[(]|\$[^$\n]+\$/
const HAS_MERMAID = /```mermaid/

function useHeavyPlugins(content: string) {
  const [, forceUpdate] = useState(0)
  const needMath = HAS_MATH.test(content)
  const needMermaid = HAS_MERMAID.test(content)

  useEffect(() => {
    let changed = false
    if (needMath && !cachedMath && !mathLoading) {
      mathLoading = true
      Promise.all([
        import("@streamdown/math"),
        import("katex/dist/katex.min.css"),
      ]).then(([mod]) => {
        cachedMath = mod.math
        mathLoading = false
        changed = true
        forceUpdate((n) => n + 1)
      })
    }
    if (needMermaid && !cachedMermaid && !mermaidLoading) {
      mermaidLoading = true
      import("@streamdown/mermaid").then((mod) => {
        cachedMermaid = mod.mermaid
        mermaidLoading = false
        if (!changed) forceUpdate((n) => n + 1)
      })
    }
  }, [needMath, needMermaid])

  return useMemo(() => {
    const p: PluginConfig = { code, cjk }
    if (cachedMath) p.math = cachedMath
    if (cachedMermaid) p.mermaid = cachedMermaid
    return p
  }, [
    // Re-memo when plugins become available
    cachedMath !== null, // eslint-disable-line react-hooks/exhaustive-deps
    cachedMermaid !== null, // eslint-disable-line react-hooks/exhaustive-deps
  ])
}

/** Word-level blurIn: each completed word gets a blur-to-clear entrance */
const streamingAnimation: AnimateOptions = {
  animation: "blurIn",
  sep: "word",
  duration: 500,
  easing: "cubic-bezier(0.22, 1, 0.36, 1)",
}

// Streamdown 默认 linkSafety 弹窗的 "Open link" 按钮调用 window.open，
// Tauri webview 不支持该行为（点击无反应），改走 open_url 命令调起系统浏览器。
const linkSafetyDisabled = { enabled: false as const }

// 桌面模式下 LLM 被 system prompt 引导把文件路径写成 `[file.ts:42](/abs/path/file.ts#L42)`
// markdown 链接，本地绝对路径走 `open_directory` Tauri 命令（系统默认应用）；
// HTTP/server 模式 `supportsLocalFileOps()` 为 false 时禁用点击，避免在 server
// 主机上误开文件。非本地链接走 `openExternalUrl`（含 `window.open` fallback）。
//
// 只识别 Unix-style `/` / `~/` 前缀：streamdown 用固定 defaultSchema 的
// rehype-sanitize，`file://` 和 Windows `C:\` 路径会在 sanitize 阶段被剥
// href，永远到不了这里，识别它们没意义还会误导读代码的人。
function isLocalPath(href: string | undefined): href is string {
  return !!href && (href.startsWith("/") || href.startsWith("~/"))
}

// 剥掉 GitHub 风格 `#L<line>` 锚点。v1 不接 IDE 协议，行号会被丢，至少
// 保证 `open::that()` 拿到的是干净路径不会失败。
function normalizeLocalPath(href: string): string {
  return href.replace(/#L\d+(-L?\d+)?$/, "")
}

const IMAGE_EXTENSIONS = new Set([
  "avif",
  "bmp",
  "gif",
  "ico",
  "jpeg",
  "jpg",
  "png",
  "svg",
  "webp",
])

const AUDIO_EXTENSIONS = new Set([
  "aac",
  "aiff",
  "flac",
  "m4a",
  "mp3",
  "ogg",
  "opus",
  "wav",
  "weba",
])

const VIDEO_EXTENSIONS = new Set([
  "avi",
  "m4v",
  "mkv",
  "mov",
  "mp4",
  "mpeg",
  "mpg",
  "ogv",
  "webm",
])

const ARCHIVE_EXTENSIONS = new Set([
  "7z",
  "bz2",
  "dmg",
  "gz",
  "rar",
  "tar",
  "tgz",
  "txz",
  "xz",
  "zip",
])

const SPREADSHEET_EXTENSIONS = new Set(["csv", "ods", "tsv", "xls", "xlsm", "xlsx"])

const DOCUMENT_EXTENSIONS = new Set([
  "doc",
  "docx",
  "log",
  "md",
  "mdx",
  "odt",
  "rtf",
  "tex",
  "txt",
])

const PRESENTATION_EXTENSIONS = new Set(["key", "odp", "ppt", "pptx"])

const CONFIG_EXTENSIONS = new Set([
  "conf",
  "config",
  "env",
  "ini",
  "lock",
  "plist",
  "properties",
  "toml",
  "yaml",
  "yml",
])

const DATA_EXTENSIONS = new Set(["json", "jsonl", "parquet", "sqlite", "sqlite3", "xml"])

const CODE_EXTENSIONS = new Set([
  "c",
  "cjs",
  "cpp",
  "cs",
  "css",
  "go",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "lua",
  "mjs",
  "py",
  "rs",
  "scss",
  "sh",
  "sql",
  "svelte",
  "swift",
  "ts",
  "tsx",
  "vue",
])

type LinkKind =
  | "anchor"
  | "archive"
  | "audio"
  | "code"
  | "config"
  | "data"
  | "document"
  | "file"
  | "image"
  | "link"
  | "mail"
  | "pdf"
  | "presentation"
  | "spreadsheet"
  | "video"
  | "web"

interface LinkIconInfo {
  Icon: LucideIcon
  kind: LinkKind
}

function hrefExtension(href: string): string | null {
  const path = href.split(/[?#]/, 1)[0] ?? ""
  const lastSegment = path.split("/").pop() ?? ""
  const dotIndex = lastSegment.lastIndexOf(".")
  if (dotIndex <= 0 || dotIndex === lastSegment.length - 1) return null
  return lastSegment.slice(dotIndex + 1).toLowerCase()
}

function linkIconForHref(href: string | undefined, local: boolean): LinkIconInfo | null {
  if (!href || href === "streamdown:incomplete-link") return null
  const extension = hrefExtension(href)
  if (extension === "pdf") return { Icon: FileText, kind: "pdf" }
  if (extension && IMAGE_EXTENSIONS.has(extension)) return { Icon: FileImage, kind: "image" }
  if (extension && AUDIO_EXTENSIONS.has(extension)) return { Icon: FileAudio, kind: "audio" }
  if (extension && VIDEO_EXTENSIONS.has(extension)) return { Icon: FileVideo, kind: "video" }
  if (extension && ARCHIVE_EXTENSIONS.has(extension)) return { Icon: FileArchive, kind: "archive" }
  if (extension && SPREADSHEET_EXTENSIONS.has(extension)) {
    return { Icon: FileSpreadsheet, kind: "spreadsheet" }
  }
  if (extension && PRESENTATION_EXTENSIONS.has(extension)) {
    return { Icon: FileType, kind: "presentation" }
  }
  if (extension && DOCUMENT_EXTENSIONS.has(extension)) return { Icon: FileType, kind: "document" }
  if (extension && CONFIG_EXTENSIONS.has(extension)) return { Icon: FileCode, kind: "config" }
  if (extension && DATA_EXTENSIONS.has(extension)) return { Icon: FileCode, kind: "data" }
  if (extension && CODE_EXTENSIONS.has(extension)) return { Icon: FileCode, kind: "code" }
  if (local) return { Icon: FileIcon, kind: "file" }
  if (href.startsWith("mailto:")) return { Icon: Mail, kind: "mail" }
  if (href.startsWith("#")) return { Icon: Hash, kind: "anchor" }
  if (/^https?:\/\//i.test(href)) return { Icon: Globe, kind: "web" }
  return { Icon: Link2, kind: "link" }
}

export function MarkdownLink({
  href,
  children,
  className,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  node: _node,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown }) {
  const isIncomplete = href === "streamdown:incomplete-link"
  const local = isLocalPath(href)
  const disabledLocal = local && !getTransport().supportsLocalFileOps()
  const linkIcon = linkIconForHref(href, local)
  const LinkIcon = linkIcon?.Icon
  // Native `title` 而非 shadcn Tooltip：Streamdown 流式消息可能渲染上百
  // anchor，包 TooltipTrigger 会爆 DOM 并破坏 anchor 组件签名。Markdown
  // 内联 disabled 提示属合理例外。
  return (
    <a
      {...rest}
      href={href}
      className={cn(
        "wrap-anywhere markdown-link font-medium",
        disabledLocal && "cursor-not-allowed opacity-70",
        className,
      )}
      title={disabledLocal ? i18next.t("common.markdownLinkLocalDisabled") : rest.title}
      data-incomplete={isIncomplete || undefined}
      data-link-kind={linkIcon?.kind}
      data-streamdown="link"
      onClick={(event) => {
        if (!href || isIncomplete) return
        event.preventDefault()
        if (disabledLocal) return
        if (local) {
          void getTransport()
            .call("open_directory", { path: normalizeLocalPath(href) })
            .catch(() => {})
        } else {
          openExternalUrl(href)
        }
      }}
    >
      {LinkIcon && <LinkIcon aria-hidden="true" className="markdown-link-icon" />}
      <span className="markdown-link-label">{children}</span>
    </a>
  )
}

const markdownComponents = { a: MarkdownLink }

interface HastNode {
  type?: string
  tagName?: string
  value?: string
  properties?: Record<string, unknown>
  children?: HastNode[]
}

const AUTOLINK_SKIP_TAGS = new Set(["a", "code", "pre", "script", "style", "kbd", "samp"])

function createAutoLinkElement(text: string, href: string): HastNode {
  return {
    type: "element",
    tagName: "a",
    properties: { href },
    children: [{ type: "text", value: text }],
  }
}

function splitTextWithAutoLinks(value: string): HastNode[] {
  const matches = findAutoLinkMatches(value)
  if (matches.length === 0) return [{ type: "text", value }]

  const nodes: HastNode[] = []
  let cursor = 0
  for (const match of matches) {
    if (match.start > cursor) {
      nodes.push({ type: "text", value: value.slice(cursor, match.start) })
    }
    nodes.push(createAutoLinkElement(match.text, match.href))
    cursor = match.end
  }
  if (cursor < value.length) {
    nodes.push({ type: "text", value: value.slice(cursor) })
  }
  return nodes
}

function linkifyHastTextNodes(node: HastNode): void {
  if (node.type !== "element" && node.type !== "root") return
  if (node.tagName && AUTOLINK_SKIP_TAGS.has(node.tagName)) return
  if (!node.children?.length) return

  const children: HastNode[] = []
  let changed = false
  for (const child of node.children) {
    if (child.type === "text" && typeof child.value === "string") {
      const split = splitTextWithAutoLinks(child.value)
      children.push(...split)
      changed ||= split.length !== 1 || split[0]?.value !== child.value
    } else {
      linkifyHastTextNodes(child)
      children.push(child)
    }
  }

  if (changed) node.children = children
}

function autolinkRehypePlugin() {
  return (tree: HastNode) => {
    linkifyHastTextNodes(tree)
  }
}

/** Start catching up when backlog exceeds this */
const CATCHUP_THRESHOLD = 60
/** Max chars per frame when catching up, prevents jarring jumps */
const MAX_STEP = 8
const STREAMING_HEIGHT_GUARD_PX = 2

function getStreamingContentHeight(el: HTMLElement): number {
  return Math.ceil(el.getBoundingClientRect().height + STREAMING_HEIGHT_GUARD_PX)
}

interface MarkdownRendererProps {
  content: string
  isStreaming?: boolean
}

export default function MarkdownRenderer({ content, isStreaming = false }: MarkdownRendererProps) {
  const plugins = useHeavyPlugins(content)

  // 外部接管 Streamdown 的 animate plugin 生命周期。Streamdown 自带的
  // `animated={AnimateOptions}` 简便用法把 plugin 实例藏在内部 useMemo，且
  // Block 组件每帧 render 会调 `setPrevContentLength(getLastRenderCharCount())`
  // —— 首次 render 时 lastRenderCharCount 还是 0，prevContentLength 被回写 0，
  // 整段已渲染内容会被当成新内容跑 blurIn。组件 unmount + remount（切会话回到
  // 流式输出中的会话 / 虚拟滚动剔出再返回视口）会重新走这条 0-baseline 路径，
  // 视觉上整段重放动画一次。
  //
  // 外部托管时：(a) 在首次 render 之前调一次 `setPrevContentLength(content.length)`，
  // 让 mount 那刻已经存在的内容全部标记 duration=0；(b) 通过 `rehypePlugins`
  // prop 把 plugin 注入 Streamdown（用 `animated={false}` 关掉内部建实例的路径，
  // 也就同时关掉 Block 内部那条覆盖 prevContentLength 的逻辑）；(c) 每帧 commit
  // 后用 effect 把上一帧 rehype 跑出的 `lastRenderCharCount` 续写为下一帧的
  // baseline（plugin rehype 跑完会自动清 prevContentLength=0，必须每帧重新设）。
  // useState lazy initializer 一次性创建 plugin 并 set mount baseline；reference
  // 跨 render 稳定，且不通过 .current 访问，避开 react-hooks/refs 规则。
  const [animatePlugin] = useState<AnimatePlugin>(() => {
    const plugin = createAnimatePlugin(streamingAnimation)
    plugin.setPrevContentLength(content.length)
    return plugin
  })

  const [displayLen, setDisplayLen] = useState(() => content.length)

  const cursorRef = useRef(content.length)
  const targetRef = useRef(content.length)
  const streamingRef = useRef(isStreaming)
  const rafRef = useRef<number | null>(null)

  // Height animation refs
  const containerRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)

  // eslint-disable-next-line react-hooks/refs -- intentional "latest value" refs read only in rAF callback
  targetRef.current = content.length
  // eslint-disable-next-line react-hooks/refs
  streamingRef.current = isStreaming

  // 每帧 commit 后把上一帧 rehype 跑出的字符数续写为下一帧的 baseline。
  // animate plugin 的 rehype 跑完会自动把 prevContentLength 清 0，因此必须
  // 每帧重新设。没有 deps：commit phase 都跑。
  useEffect(() => {
    const count = animatePlugin.getLastRenderCharCount()
    if (count > 0) animatePlugin.setPrevContentLength(count)
  })

  // Non-streaming (history): show full content immediately
  useEffect(() => {
    if (!isStreaming && rafRef.current === null) {
      cursorRef.current = content.length
      setDisplayLen(content.length)
    }
  }, [isStreaming, content.length])

  // rAF loop: +1 char per frame, continues draining after stream ends (no jump)
  useEffect(() => {
    if (!isStreaming) return
    if (rafRef.current !== null) return

    const tick = () => {
      const cursor = cursorRef.current
      const target = targetRef.current

      if (cursor >= target && !streamingRef.current) {
        rafRef.current = null
        return
      }

      if (cursor < target) {
        const backlog = target - cursor
        const step = backlog > CATCHUP_THRESHOLD ? Math.min(Math.ceil(backlog * 0.1), MAX_STEP) : 1
        const next = Math.min(cursor + step, target)
        cursorRef.current = next
        setDisplayLen(next)
      }

      rafRef.current = requestAnimationFrame(tick)
    }

    rafRef.current = requestAnimationFrame(tick)
  }, [isStreaming])

  // Smooth height transition: mount ResizeObserver once when streaming starts,
  // let it detect height changes on its own to avoid breaking CSS transitions
  useLayoutEffect(() => {
    const container = containerRef.current
    const contentEl = contentRef.current
    if (!container || !contentEl || !isStreaming) {
      if (containerRef.current) containerRef.current.style.height = ""
      return
    }

    container.style.height = `${getStreamingContentHeight(contentEl)}px`

    const observer = new ResizeObserver(() => {
      const h = getStreamingContentHeight(contentEl)
      if (container.style.height !== `${h}px`) {
        container.style.height = `${h}px`
      }
    })
    observer.observe(contentEl)

    return () => {
      observer.disconnect()
      container.style.height = ""
    }
  }, [isStreaming])

  useEffect(() => {
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current)
        rafRef.current = null
      }
    }
  }, [])

  if (!content) return null

  const revealing = displayLen < content.length
  const displayContent = revealing ? content.slice(0, displayLen) : content
  const isActive = isStreaming || revealing

  // 流式期间把外部 animate plugin 注入 rehype 链尾；静态历史消息直接复用
  // streamdown 默认 rehype（raw/sanitize/harden 安全基线）。`animated={false}`
  // 关掉 streamdown 内部建实例的路径，避免与外部 plugin 重复跑。
  const defaultRehypePluginList = Object.values(defaultRehypePlugins)
  const rehypePlugins = isActive
    ? [...defaultRehypePluginList, autolinkRehypePlugin, animatePlugin.rehypePlugin]
    : [...defaultRehypePluginList, autolinkRehypePlugin]

  return (
    <div
      ref={containerRef}
      className={isActive ? "streaming-height markdown-content" : "markdown-content"}
    >
      <div ref={contentRef}>
        <Streamdown
          animated={false}
          plugins={plugins}
          isAnimating={isActive}
          parseIncompleteMarkdown={isActive}
          rehypePlugins={rehypePlugins}
          linkSafety={linkSafetyDisabled}
          components={markdownComponents}
        >
          {displayContent}
        </Streamdown>
      </div>
    </div>
  )
}
