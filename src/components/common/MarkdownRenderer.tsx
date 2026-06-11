import {
  memo,
  useState,
  useEffect,
  useMemo,
  type AnchorHTMLAttributes,
  type ComponentProps,
  type ImgHTMLAttributes,
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
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  FolderOpen,
  Globe,
  Hash,
  Link2,
  Mail,
  type LucideIcon,
} from "lucide-react"
import "streamdown/styles.css"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"
import { findAutoLinkMatches } from "@/lib/autoLink"
import { shouldRenderAsBareJson } from "./markdownJson"
import { useFileActions } from "@/components/chat/files/useFileActions"
import { FileContextMenu } from "@/components/chat/files/FileActionMenu"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import { SkillMentionChip } from "@/components/chat/skill-mention/SkillMentionChip"
import { isSkillMentionName, skillNameFromHref } from "@/components/chat/skill-mention/skillTokens"

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

/** Char-level fadeIn: each newly-appended character fades in, staggered for a
 *  smooth flowing reveal. The animate plugin animates only the new tail (chars
 *  below the prev-render baseline get duration=0), so no external slice
 *  typewriter is needed — content is handed to Streamdown in full. */
const streamingAnimation: AnimateOptions = {
  animation: "fadeIn",
  sep: "char",
  duration: 200,
  stagger: 16,
  easing: "cubic-bezier(0.22, 1, 0.36, 1)",
}

// char 级 animate 把每个字符 wrap 成一个 `<span>`，活跃块每帧重新 wrap 全部字符
// （baseline 字符也是 duration=0 的 span）——O(n) span/帧，内容越长越爆。无空格的
// 长中文尤其致命（一段 = 上千 span）。超过该阈值就关掉逐字动画：仍按流式渲染、
// 保留 incomplete-markdown 处理，只是不再逐字渐显，长回复换来平滑出字。
const ANIMATE_MAX_CHARS = 4000

// Streamdown 默认 linkSafety 弹窗的 "Open link" 按钮调用 window.open，
// Tauri webview 不支持该行为（点击无反应），改走 open_url 命令调起系统浏览器。
const linkSafetyDisabled = { enabled: false as const }

// 桌面模式下 LLM 被 system prompt 引导把文件路径写成 `[file.ts:42](/abs/path/file.ts#L42)`
// markdown 链接，本地绝对路径走 `open_directory` Tauri 命令（系统默认应用）；
// HTTP/server 模式 `supportsLocalFileOps()` 为 false 时禁用点击，避免在 server
// 主机上误开文件。非本地链接走 `openExternalUrl`（含 `window.open` fallback）。
//
// 只承诺 Unix-style `/` / `~/` 前缀：streamdown 用固定 defaultSchema 的
// rehype-sanitize，Windows `C:\` 路径会在 sanitize 阶段被剥 href，永远
// 到不了这里。Tauri/WebKit 有时会把 `/Users/...` 暴露成同源绝对 URL
// (`http://tauri.localhost/Users/...` / dev server origin)，所以点击前统一
// 还原成本机路径。
function localPathFromHref(href: string | undefined): string | null {
  if (!href) return null
  if (href.startsWith("/") || href.startsWith("~/")) return normalizeLocalPath(href)

  try {
    const url = new URL(href)
    const sameOrigin =
      typeof window !== "undefined" &&
      window.location?.origin &&
      url.origin === window.location.origin
    if ((sameOrigin || url.hostname === "tauri.localhost") && url.pathname.startsWith("/")) {
      return normalizeLocalPath(`${url.pathname}${url.hash}`)
    }
    if (url.protocol === "file:" && url.pathname.startsWith("/")) {
      return normalizeLocalPath(`${url.pathname}${url.hash}`)
    }
  } catch {
    // Not an absolute URL; fall through to regular link handling.
  }

  return null
}

// 剥掉 GitHub 风格 `#L<line>` 锚点。v1 不接 IDE 协议，行号会被丢，至少
// 保证 `open::that()` 拿到的是干净路径不会失败。
function normalizeLocalPath(href: string): string {
  const withoutLineAnchor = href.replace(/#L\d+(-L?\d+)?$/, "")
  try {
    return decodeURI(withoutLineAnchor)
  } catch {
    return withoutLineAnchor
  }
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
  | "folder"
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
  if (local) return { Icon: FolderOpen, kind: "folder" }
  if (href.startsWith("mailto:")) return { Icon: Mail, kind: "mail" }
  if (href.startsWith("#")) return { Icon: Hash, kind: "anchor" }
  if (/^https?:\/\//i.test(href)) return { Icon: Globe, kind: "web" }
  return { Icon: Link2, kind: "link" }
}

type MarkdownAnchorProps = AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown }
type MarkdownImageProps = ImgHTMLAttributes<HTMLImageElement> & { node?: unknown }

function MarkdownImage({
  alt,
  className,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  node: _node,
  ...rest
}: MarkdownImageProps) {
  return (
    <span className="markdown-image-wrapper" data-streamdown="image-wrapper">
      <img
        {...rest}
        alt={alt ?? ""}
        className={cn("markdown-image", className)}
        data-streamdown="image"
      />
    </span>
  )
}

export function MarkdownLink({
  href,
  children,
  className,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  node: _node,
  ...rest
}: MarkdownAnchorProps) {
  const isIncomplete = href === "streamdown:incomplete-link"
  // `@skill` mentions are `[@label](#skill:<name>)` links — render the same rose
  // chip the composer shows, not the raw link. Fragment href survives sanitize.
  const skillName = isIncomplete ? null : skillNameFromHref(href)
  if (skillName && isSkillMentionName(skillName)) {
    return <SkillMentionChip name={skillName} />
  }
  const localPath = isIncomplete ? null : localPathFromHref(href)
  // Local file links follow the unified file-operation policy (preview / open /
  // download by kind × mode) + a right-click menu — but ONLY local links pay the
  // useFileActions hook + Radix ContextMenu cost. External links (the vast
  // majority, and the file's perf concern: a streamed message renders hundreds
  // of anchors) render as a plain anchor with no hooks. So MarkdownLink itself
  // calls no hooks and dispatches by kind.
  if (localPath) {
    return (
      <MarkdownFileLink localPath={localPath} href={href} className={className} {...rest}>
        {children}
      </MarkdownFileLink>
    )
  }
  const linkIcon = linkIconForHref(href, false)
  const LinkIcon = linkIcon?.Icon
  // Native `title` 而非 shadcn Tooltip：Streamdown 流式消息可能渲染上百 anchor，
  // 包 TooltipTrigger 会爆 DOM 并破坏 anchor 组件签名。
  return (
    <a
      {...rest}
      href={href}
      className={cn("wrap-anywhere markdown-link font-medium", className)}
      data-incomplete={isIncomplete || undefined}
      data-link-kind={linkIcon?.kind}
      data-streamdown="link"
      onClick={(event) => {
        if (!href || isIncomplete) return
        event.preventDefault()
        openExternalUrl(href)
      }}
    >
      {LinkIcon && <LinkIcon aria-hidden="true" className="markdown-link-icon" />}
      <span className="markdown-link-label">{children}</span>
    </a>
  )
}

/** Local-path Markdown link: unified primary-click + right-click action menu.
 *  Split out so only local links instantiate the hook + ContextMenu (external
 *  links stay zero-cost). `memo` + a memoized `target` keep streaming re-renders
 *  from re-running the resolution per frame. */
const MarkdownFileLink = memo(function MarkdownFileLink({
  localPath,
  href,
  className,
  children,
  ...rest
}: MarkdownAnchorProps & { localPath: string }) {
  const target = useMemo<PreviewTarget>(
    () => ({ kind: "path", path: localPath, name: basename(localPath) }),
    [localPath],
  )
  const { primary, run } = useFileActions(target)
  const linkIcon = linkIconForHref(href, true)
  const LinkIcon = linkIcon?.Icon
  return (
    <FileContextMenu target={target}>
      <a
        {...rest}
        href={href}
        className={cn("wrap-anywhere markdown-link font-medium", className)}
        data-link-kind={linkIcon?.kind}
        data-streamdown="link"
        onClick={(event) => {
          if (!href) return
          event.preventDefault()
          run(primary)
        }}
      >
        {LinkIcon && <LinkIcon aria-hidden="true" className="markdown-link-icon" />}
        <span className="markdown-link-label">{children}</span>
      </a>
    </FileContextMenu>
  )
})

const markdownComponents = { a: MarkdownLink, img: MarkdownImage }

export function MarkdownStreamdown({
  children,
  ...props
}: Omit<ComponentProps<typeof Streamdown>, "components">) {
  return (
    <Streamdown {...props} components={markdownComponents}>
      {children}
    </Streamdown>
  )
}

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

interface MarkdownRendererProps {
  content: string
  isStreaming?: boolean
}

function BareJsonRenderer({ content, isStreaming }: MarkdownRendererProps) {
  return (
    <div className="markdown-content markdown-json-content">
      <pre
        data-hope-bare-json
        data-streaming={isStreaming || undefined}
        className="hope-bare-json-block"
      >
        <code>{content}</code>
      </pre>
    </div>
  )
}

function StreamdownMarkdownRenderer({ content, isStreaming = false }: MarkdownRendererProps) {
  const plugins = useHeavyPlugins(content)

  // 外部接管 Streamdown 的 animate plugin 生命周期。Streamdown 自带的
  // `animated={AnimateOptions}` 简便用法把 plugin 实例藏在内部 useMemo，且
  // Block 组件每帧 render 会调 `setPrevContentLength(getLastRenderCharCount())`
  // —— 首次 render 时 lastRenderCharCount 还是 0，prevContentLength 被回写 0，
  // 整段已渲染内容会被当成新内容跑一遍入场动画。组件 unmount + remount（切会话回到
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

  // 每帧 commit 后把上一帧 rehype 跑出的字符数续写为下一帧的 baseline。
  // animate plugin 的 rehype 跑完会自动把 prevContentLength 清 0，因此必须
  // 每帧重新设。没有 deps：commit phase 都跑。
  useEffect(() => {
    const count = animatePlugin.getLastRenderCharCount()
    if (count > 0) animatePlugin.setPrevContentLength(count)
  })

  // content 全量交给 Streamdown：它按 block 分块 memo，只重解析变化的末块；
  // 新增尾部由 animate plugin 按 stagger 逐字错峰渐显，无需外部 slice 打字机。
  const isActive = isStreaming

  // 流式期间把外部 animate plugin 注入 rehype 链尾；静态历史消息直接复用
  // streamdown 默认 rehype（raw/sanitize/harden 安全基线）。`animated={false}`
  // 关掉 streamdown 内部建实例的路径，避免与外部 plugin 重复跑。
  //
  // 超长内容关掉逐字动画（见 ANIMATE_MAX_CHARS）；仍是流式渲染，只是不挂 animate
  // plugin。布尔值跨帧只在跨阈值时翻转一次，不破坏下面 useMemo 的稳定性。
  const animateActive = isActive && content.length <= ANIMATE_MAX_CHARS

  // **必须 memo**：Streamdown 的 Block memo 比较器要求 `rehypePlugins` 引用相等
  // 才跳过重渲染（vendored chunk 里 `e.rehypePlugins!==t.rehypePlugins` 即判失效），
  // 且它把该数组 `useMemo([a,...])` 原样转发给每个 block。若每帧新建数组，流式每帧
  // 都会让**所有** block（含已定稿的长 prose / Shiki 代码块）全量重渲染——内容越长
  // 越卡。稳定引用后，每帧只重渲染正在增长的末块。
  const rehypePlugins = useMemo(() => {
    const base = [...Object.values(defaultRehypePlugins), autolinkRehypePlugin]
    return animateActive ? [...base, animatePlugin.rehypePlugin] : base
  }, [animateActive, animatePlugin])

  if (!content) return null

  return (
    <div className="markdown-content">
      <div>
        <Streamdown
          animated={false}
          plugins={plugins}
          isAnimating={isActive}
          parseIncompleteMarkdown={isActive}
          rehypePlugins={rehypePlugins}
          linkSafety={linkSafetyDisabled}
          components={markdownComponents}
        >
          {content}
        </Streamdown>
      </div>
    </div>
  )
}

// memo：props 仅 `content`(string) + `isStreaming`(bool)，默认浅比较即可。
// 流式 bubble 每帧重建所有 text block 的 MarkdownRenderer 元素，已定稿 block 的
// 这两个 prop 都稳定——memo 让它们整体跳过 render，避免无谓的 Streamdown 全量
// re-lex，只剩正在增长的末块真正重渲染。
function MarkdownRenderer({ content, isStreaming = false }: MarkdownRendererProps) {
  if (shouldRenderAsBareJson(content)) {
    return <BareJsonRenderer content={content} isStreaming={isStreaming} />
  }
  return <StreamdownMarkdownRenderer content={content} isStreaming={isStreaming} />
}

export default memo(MarkdownRenderer)
