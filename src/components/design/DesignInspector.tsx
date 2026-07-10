/**
 * 属性检视器（D1 可视化微调的可见半边）。
 *
 * 接收 iframe bridge 回传的选中元素，提供分区控件（文本/颜色/排版/间距/圆角）：
 * 交互时**即时预览**（回调驱动 iframe live style），交互结束**提交**回写源码。
 * 控件是纯受控组件，父层负责 preview / commit 两条通道。
 */

import { useState } from "react"
import { useTranslation } from "react-i18next"
import { X, AlignLeft, AlignCenter, AlignRight, Link2, Link, Unlink, ImageUp, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { Slider } from "@/components/ui/slider"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { DesignSelectedElement } from "@/types/design"

interface Props {
  selected: DesignSelectedElement
  onLiveStyle: (prop: string, value: string) => void
  onCommitStyle: (prop: string, value: string) => void
  onLiveText: (text: string) => void
  onCommitText: (text: string) => void
  /** B5：href/src/alt 即时预览（ds_preview_attr）。 */
  onLiveAttr: (attr: string, value: string) => void
  /** B5：href/src/alt 提交回写（确定性 patch）。 */
  onCommitAttr: (attr: string, value: string) => void
  /** B5：选本地图 → data-uri（桌面/HTTP 统一）；返回 null = 取消/失败。 */
  onPickImage: () => Promise<string | null>
  onClose: () => void
}

/** 内置字体栈（Wave 3-⑫）。name 为字体本名（专有名词、无需 i18n）。 */
const FONT_STACKS: { name: string; stack: string }[] = [
  { name: "System", stack: "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif" },
  { name: "Helvetica", stack: "'Helvetica Neue', Helvetica, Arial, sans-serif" },
  { name: "Inter", stack: "Inter, 'Segoe UI', sans-serif" },
  { name: "Georgia", stack: "Georgia, 'Times New Roman', serif" },
  { name: "Menlo", stack: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace" },
]

const hex2 = (n: number) => Math.max(0, Math.min(255, n || 0)).toString(16).padStart(2, "0")

function rgbStrToHex(inner: string): string {
  const [r, g, b] = inner.split(",").map((x) => parseInt(x.trim(), 10))
  return `#${hex2(r)}${hex2(g)}${hex2(b)}`
}

/**
 * Any CSS color (`#rgb` / `#rrggbb` / `rgb()` / `rgba()` / named / `hsl()`) →
 * `#rrggbb`, which is all `<input type="color">` accepts. Named / hsl / 3-digit are
 * resolved via a canvas (best-effort) instead of collapsing to black, so the swatch
 * reflects the real color and a stray drag can't silently repaint an element black.
 */
function toHex(v: string): string {
  const s = (v || "").trim()
  if (!s) return "#000000"
  if (/^#[0-9a-fA-F]{6}$/.test(s)) return s.toLowerCase()
  if (/^#[0-9a-fA-F]{3}$/.test(s)) {
    const [r, g, b] = [s[1], s[2], s[3]]
    return `#${r}${r}${g}${g}${b}${b}`.toLowerCase()
  }
  const m = s.match(/rgba?\(([^)]+)\)/)
  if (m) return rgbStrToHex(m[1])
  try {
    const ctx = document.createElement("canvas").getContext("2d")
    if (ctx) {
      ctx.fillStyle = "#000000"
      ctx.fillStyle = s // invalid input leaves the previous (#000000)
      const resolved = ctx.fillStyle
      if (/^#[0-9a-fA-F]{6}$/.test(resolved)) return resolved.toLowerCase()
      const rm = resolved.match(/rgba?\(([^)]+)\)/)
      if (rm) return rgbStrToHex(rm[1])
    }
  } catch {
    /* ignore — fall through */
  }
  return "#000000"
}

function px(v: string): number {
  return Math.round(parseFloat(v) || 0)
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="border-b px-3 py-3">
      <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      <div className="space-y-2">{children}</div>
    </div>
  )
}

function ColorRow({
  label,
  prop,
  value,
  onLive,
  onCommit,
}: {
  label: string
  prop: string
  value: string
  onLive: (prop: string, v: string) => void
  onCommit: (prop: string, v: string) => void
}) {
  const hex = toHex(value)
  // 可编辑 hex 手输（Wave 3-⑫）：粘贴品牌色不必再回对话。非法回退当前值。渲染期 prev-prop
  // 同步（与 NumberRow 一致，避免 setState-in-effect 级联渲染）。
  const [draft, setDraft] = useState(hex)
  const [prevHex, setPrevHex] = useState(hex)
  if (hex !== prevHex) {
    setPrevHex(hex)
    setDraft(hex)
  }
  const commitDraft = () => {
    let v = draft.trim()
    if (v && !v.startsWith("#")) v = `#${v}`
    if (/^#[0-9a-fA-F]{3}$/.test(v) || /^#[0-9a-fA-F]{6}$/.test(v)) onCommit(prop, v.toLowerCase())
    else setDraft(hex)
  }
  return (
    <label className="flex items-center justify-between gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="flex items-center gap-1.5">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commitDraft}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur()
          }}
          className="h-6 w-[72px] px-1.5 font-mono text-[11px]"
        />
        <input
          type="color"
          value={hex}
          onInput={(e) => onLive(prop, (e.target as HTMLInputElement).value)}
          onChange={(e) => onCommit(prop, e.target.value)}
          className="h-6 w-8 cursor-pointer rounded border bg-transparent p-0"
        />
      </span>
    </label>
  )
}

/** 四向数值控件（Wave 3-⑫）：内 / 外边距逐边可调 + 联动锁（锁时改一边=改全等）。 */
function QuadRow({
  label,
  prop,
  styles,
  onCommit,
}: {
  label: string
  prop: "padding" | "margin"
  styles: Record<string, string>
  onCommit: (prop: string, v: string) => void
}) {
  const sides = ["top", "right", "bottom", "left"] as const
  const vals = sides.map((side) => px(styles[`${prop}-${side}`] || styles[prop] || "0"))
  const [linked, setLinked] = useState(vals.every((v) => v === vals[0]))
  const set = (i: number, raw: string) => {
    const v = Math.round(parseFloat(raw) || 0)
    if (linked) sides.forEach((side) => onCommit(`${prop}-${side}`, `${v}px`))
    else onCommit(`${prop}-${sides[i]}`, `${v}px`)
  }
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between">
        <span className="text-sm text-muted-foreground">{label}</span>
        <IconTip label={linked ? label : label} side="left">
          <button
            type="button"
            onClick={() => setLinked((v) => !v)}
            className="flex h-5 w-5 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            {linked ? <Link className="h-3.5 w-3.5" /> : <Unlink className="h-3.5 w-3.5" />}
          </button>
        </IconTip>
      </div>
      <div className="grid grid-cols-4 gap-1">
        {sides.map((side, i) => (
          <Input
            key={side}
            type="number"
            value={vals[i]}
            onChange={(e) => set(i, e.target.value)}
            className="h-7 px-1 text-center text-xs"
            aria-label={side}
          />
        ))}
      </div>
    </div>
  )
}

function NumberRow({
  label,
  prop,
  value,
  suffix = "px",
  onCommit,
}: {
  label: string
  prop: string
  value: number
  suffix?: string
  onCommit: (prop: string, v: string) => void
}) {
  const [v, setV] = useState(String(value))
  // Sync local input when the selected element's value changes (render-phase
  // prev-prop tracking — avoids setState-in-effect cascading renders).
  const [prevValue, setPrevValue] = useState(value)
  if (value !== prevValue) {
    setPrevValue(value)
    setV(String(value))
  }
  // 脏值守卫：未改不 commit（防聚焦+失焦把 computed 值原样写回源码，review #4）。
  // NaN / 空守卫（B0-7）：非法输入回填原值、绝不静默 commit 成 0 抹掉尺寸；负值仍合法（不钳）。
  const commit = () => {
    const n = parseFloat(v)
    if (!Number.isFinite(n)) {
      setV(String(value))
      return
    }
    if (n !== value) onCommit(prop, `${n}${suffix}`)
  }
  return (
    <label className="flex items-center justify-between gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <Input
        type="number"
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit()
        }}
        className="h-7 w-20 text-xs"
      />
    </label>
  )
}

/** 自由 CSS 值输入（宽/高等，允许 `auto` / `%` / `px`）；渲染期 prev-prop 同步。 */
function TextRow({
  label,
  prop,
  value,
  placeholder,
  onCommit,
}: {
  label: string
  prop: string
  value: string
  placeholder?: string
  onCommit: (prop: string, v: string) => void
}) {
  const [v, setV] = useState(value)
  const [prev, setPrev] = useState(value)
  if (value !== prev) {
    setPrev(value)
    setV(value)
  }
  // 脏值守卫：未改不 commit（尺寸值来自 computed，聚焦+失焦不该把 `1440px` 写回一个 auto 元素，review #4）。
  const commit = () => {
    if (v.trim() !== value.trim()) onCommit(prop, v.trim())
  }
  return (
    <label className="flex items-center justify-between gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <Input
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit()
        }}
        className="h-7 w-24 text-xs"
      />
    </label>
  )
}

/** 带标签的枚举下拉（display / border-style 等）。 */
function SelectRow({
  label,
  prop,
  value,
  options,
  onCommit,
}: {
  label: string
  prop: string
  value: string
  options: [string, string][]
  onCommit: (prop: string, v: string) => void
}) {
  return (
    <div className="flex items-center justify-between gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <Select value={value} onValueChange={(v) => onCommit(prop, v)}>
        <SelectTrigger className="h-7 w-28 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([val, lbl]) => (
            <SelectItem key={val} value={val} className="text-xs">
              {lbl}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

/** 不透明度滑杆：本地拖动 state（受控 Slider 拇指随指针走）+ 渲染期 prev-prop 同步。 */
function OpacityRow({
  value,
  onLive,
  onCommit,
}: {
  value: number
  onLive: (prop: string, v: string) => void
  onCommit: (prop: string, v: string) => void
}) {
  const { t } = useTranslation()
  const [local, setLocal] = useState(value)
  const [prev, setPrev] = useState(value)
  if (value !== prev) {
    setPrev(value)
    setLocal(value)
  }
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-sm">
        <span className="text-muted-foreground">{t("design.insp.opacity", "不透明度")}</span>
        <span className="font-mono text-xs text-muted-foreground">{Math.round(local * 100)}%</span>
      </div>
      <Slider
        min={0}
        max={1}
        step={0.01}
        value={[local]}
        onValueChange={(v) => {
          setLocal(v[0])
          onLive("opacity", String(v[0]))
        }}
        onValueCommit={(v) => onCommit("opacity", String(v[0]))}
      />
    </div>
  )
}

export default function DesignInspector({
  selected,
  onLiveStyle,
  onCommitStyle,
  onLiveText,
  onCommitText,
  onLiveAttr,
  onCommitAttr,
  onPickImage,
  onClose,
}: Props) {
  const { t } = useTranslation()
  const s = selected.styles
  const [text, setText] = useState(selected.text)
  // B5：链接 / 图片属性本地草稿。
  const [href, setHref] = useState(selected.attrs?.href ?? "")
  const [imgSrc, setImgSrc] = useState(selected.attrs?.src ?? "")
  const [imgAlt, setImgAlt] = useState(selected.attrs?.alt ?? "")
  const [uploading, setUploading] = useState(false)
  // 草稿跟随**外部值变化**（不只 oid）——否则 undo/redo 改了同一元素的 text/href/src/alt 后，输入框
  // 还停在旧草稿，一次失焦会把旧值重新提交、把 undo 抵消（review 修复）。渲染期 prev-prop 对账，
  // 只重置真正变了的字段；打字期（onLive* 不改 selected）外部值不变故不与用户输入相争。
  const extText = selected.text
  const extHref = selected.attrs?.href ?? ""
  const extSrc = selected.attrs?.src ?? ""
  const extAlt = selected.attrs?.alt ?? ""
  const [prevExt, setPrevExt] = useState({
    text: extText,
    href: extHref,
    src: extSrc,
    alt: extAlt,
  })
  if (
    prevExt.text !== extText ||
    prevExt.href !== extHref ||
    prevExt.src !== extSrc ||
    prevExt.alt !== extAlt
  ) {
    if (prevExt.text !== extText) setText(extText)
    if (prevExt.href !== extHref) setHref(extHref)
    if (prevExt.src !== extSrc) setImgSrc(extSrc)
    if (prevExt.alt !== extAlt) setImgAlt(extAlt)
    setPrevExt({ text: extText, href: extHref, src: extSrc, alt: extAlt })
  }

  const align = s["text-align"] || "left"
  const display = s["display"] || "block"
  const isFlexish = display === "flex" || display === "inline-flex" || display === "grid"
  // 字体族匹配到内置栈（Wave 3-⑫）：computed font-family 含某栈首字体即选中，否则默认「系统」。
  const rawFF = (s["font-family"] || "").toLowerCase()
  const fontFamilyKey =
    FONT_STACKS.find((f) => rawFF.includes(f.name.toLowerCase()))?.stack ?? FONT_STACKS[0].stack
  const opacity = parseFloat(s["opacity"] || "1")

  return (
    <div className="flex h-full w-72 shrink-0 flex-col overflow-y-auto border-l bg-background">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b px-3">
        <span className="font-mono text-xs font-semibold text-primary">
          &lt;{selected.tag}&gt;
        </span>
        <span className="text-[11px] text-muted-foreground">#{selected.oid}</span>
        <IconTip label={t("common.close", "关闭")} side="bottom">
          <Button variant="ghost" size="icon" className="ml-auto h-6 w-6" onClick={onClose}>
            <X className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      </div>

      {selected.isLeaf && (
        <Section title={t("design.insp.text", "文本")}>
          <Textarea
            value={text}
            onChange={(e) => {
              setText(e.target.value)
              onLiveText(e.target.value)
            }}
            onBlur={() => {
              // dirty-guard：文本未变不提交，避免每次失焦都产生冗余 patch + 新版本（review Frontend-3）。
              if (text !== (selected.text ?? "")) onCommitText(text)
            }}
            rows={2}
            className="resize-none"
          />
        </Section>
      )}

      {/* B5：链接编辑（<a href>） */}
      {selected.tag === "a" && (
        <Section title={t("design.insp.link", "链接")}>
          <div className="space-y-1.5">
            <label className="flex items-center gap-1 text-[11px] text-muted-foreground">
              <Link2 className="h-3 w-3" />
              {t("design.insp.href", "链接地址")}
            </label>
            <Input
              value={href}
              onChange={(e) => {
                setHref(e.target.value)
                onLiveAttr("href", e.target.value)
              }}
              onBlur={() => {
                if (href !== (selected.attrs?.href ?? "")) onCommitAttr("href", href)
              }}
              placeholder="https://…"
              className="h-8 text-xs"
            />
          </div>
        </Section>
      )}

      {/* B5：图片编辑（<img src/alt> + 本地上传→data-uri） */}
      {selected.tag === "img" && (
        <Section title={t("design.insp.image", "图片")}>
          <div className="space-y-2">
            <div className="space-y-1.5">
              <label className="text-[11px] text-muted-foreground">
                {t("design.insp.imageSrc", "图片地址")}
              </label>
              <Input
                value={imgSrc}
                onChange={(e) => {
                  setImgSrc(e.target.value)
                  onLiveAttr("src", e.target.value)
                }}
                onBlur={() => {
                  if (imgSrc !== (selected.attrs?.src ?? "")) onCommitAttr("src", imgSrc)
                }}
                placeholder="https://… / data:image/…"
                className="h-8 text-xs"
              />
            </div>
            <Button
              variant="outline"
              size="sm"
              className="h-8 w-full gap-1.5 text-xs"
              disabled={uploading}
              onClick={async () => {
                setUploading(true)
                try {
                  const dataUri = await onPickImage()
                  if (dataUri) {
                    setImgSrc(dataUri)
                    onCommitAttr("src", dataUri)
                  }
                } finally {
                  setUploading(false)
                }
              }}
            >
              {uploading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <ImageUp className="h-3.5 w-3.5" />
              )}
              {t("design.insp.uploadImage", "上传本地图片")}
            </Button>
            <div className="space-y-1.5">
              <label className="text-[11px] text-muted-foreground">
                {t("design.insp.imageAlt", "替代文本 (alt)")}
              </label>
              <Input
                value={imgAlt}
                onChange={(e) => setImgAlt(e.target.value)}
                onBlur={() => {
                  if (imgAlt !== (selected.attrs?.alt ?? "")) onCommitAttr("alt", imgAlt)
                }}
                placeholder={t("design.insp.imageAltHint", "图片描述")}
                className="h-8 text-xs"
              />
            </div>
          </div>
        </Section>
      )}

      <Section title={t("design.insp.color", "颜色")}>
        <ColorRow
          label={t("design.insp.textColor", "文字")}
          prop="color"
          value={s["color"] || ""}
          onLive={onLiveStyle}
          onCommit={onCommitStyle}
        />
        <ColorRow
          label={t("design.insp.bgColor", "背景")}
          prop="background-color"
          value={s["background-color"] || ""}
          onLive={onLiveStyle}
          onCommit={onCommitStyle}
        />
      </Section>

      <Section title={t("design.insp.typography", "排版")}>
        <SelectRow
          label={t("design.insp.fontFamily", "字体")}
          prop="font-family"
          value={fontFamilyKey}
          options={FONT_STACKS.map((f) => [f.stack, f.name] as [string, string])}
          onCommit={onCommitStyle}
        />
        <NumberRow
          label={t("design.insp.fontSize", "字号")}
          prop="font-size"
          value={px(s["font-size"] || "16")}
          onCommit={onCommitStyle}
        />
        <NumberRow
          label={t("design.insp.fontWeight", "字重")}
          prop="font-weight"
          value={parseInt(s["font-weight"] || "400", 10)}
          suffix=""
          onCommit={onCommitStyle}
        />
        <TextRow
          label={t("design.insp.lineHeight", "行高")}
          prop="line-height"
          value={s["line-height"] === "normal" ? "" : s["line-height"] || ""}
          placeholder="1.5 / 24px"
          onCommit={onCommitStyle}
        />
        <TextRow
          label={t("design.insp.letterSpacing", "字距")}
          prop="letter-spacing"
          value={s["letter-spacing"] === "normal" ? "" : s["letter-spacing"] || ""}
          placeholder="normal"
          onCommit={onCommitStyle}
        />
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground">{t("design.insp.align", "对齐")}</span>
          <div className="flex gap-0.5">
            {(
              [
                ["left", AlignLeft],
                ["center", AlignCenter],
                ["right", AlignRight],
              ] as const
            ).map(([a, Icon]) => (
              <Button
                key={a}
                variant={align === a ? "default" : "ghost"}
                size="icon"
                className="h-6 w-6"
                onClick={() => onCommitStyle("text-align", a)}
              >
                <Icon className="h-3.5 w-3.5" />
              </Button>
            ))}
          </div>
        </div>
      </Section>

      <Section title={t("design.insp.spacing", "间距与圆角")}>
        <QuadRow
          label={t("design.insp.padding", "内边距")}
          prop="padding"
          styles={s}
          onCommit={onCommitStyle}
        />
        <QuadRow
          label={t("design.insp.margin", "外边距")}
          prop="margin"
          styles={s}
          onCommit={onCommitStyle}
        />
        <NumberRow
          label={t("design.insp.radius", "圆角")}
          prop="border-radius"
          value={px(s["border-radius"] || "0")}
          onCommit={onCommitStyle}
        />
      </Section>

      <Section title={t("design.insp.layout", "布局")}>
        <SelectRow
          label={t("design.insp.display", "显示")}
          prop="display"
          value={display}
          options={[
            ["block", "block"],
            ["flex", "flex"],
            ["inline-flex", "inline-flex"],
            ["grid", "grid"],
            ["inline-block", "inline-block"],
            ["none", "none"],
          ]}
          onCommit={onCommitStyle}
        />
        {isFlexish && (
          <>
            <SelectRow
              label={t("design.insp.alignItems", "纵向对齐")}
              prop="align-items"
              value={s["align-items"] || "stretch"}
              options={[
                ["flex-start", t("design.insp.start", "起始")],
                ["center", t("design.insp.center", "居中")],
                ["flex-end", t("design.insp.end", "末尾")],
                ["stretch", t("design.insp.stretch", "拉伸")],
                ["baseline", t("design.insp.baseline", "基线")],
              ]}
              onCommit={onCommitStyle}
            />
            <SelectRow
              label={t("design.insp.justify", "横向分布")}
              prop="justify-content"
              value={s["justify-content"] || "flex-start"}
              options={[
                ["flex-start", t("design.insp.start", "起始")],
                ["center", t("design.insp.center", "居中")],
                ["flex-end", t("design.insp.end", "末尾")],
                ["space-between", t("design.insp.between", "两端")],
                ["space-around", t("design.insp.around", "环绕")],
                ["space-evenly", t("design.insp.evenly", "均匀")],
              ]}
              onCommit={onCommitStyle}
            />
            <NumberRow
              label={t("design.insp.gap", "间隙")}
              prop="gap"
              value={px(s["gap"] || "0")}
              onCommit={onCommitStyle}
            />
          </>
        )}
      </Section>

      <Section title={t("design.insp.size", "尺寸")}>
        <TextRow
          label={t("design.insp.width", "宽")}
          prop="width"
          value={s["width"] || ""}
          placeholder="auto"
          onCommit={onCommitStyle}
        />
        <TextRow
          label={t("design.insp.height", "高")}
          prop="height"
          value={s["height"] || ""}
          placeholder="auto"
          onCommit={onCommitStyle}
        />
        <TextRow
          label={t("design.insp.maxWidth", "最大宽")}
          prop="max-width"
          value={s["max-width"] || ""}
          placeholder="none"
          onCommit={onCommitStyle}
        />
        <TextRow
          label={t("design.insp.minHeight", "最小高")}
          prop="min-height"
          value={s["min-height"] || ""}
          placeholder="0"
          onCommit={onCommitStyle}
        />
      </Section>

      <Section title={t("design.insp.stroke", "描边")}>
        <NumberRow
          label={t("design.insp.borderWidth", "边框宽")}
          prop="border-width"
          value={px(s["border-width"] || "0")}
          onCommit={onCommitStyle}
        />
        <SelectRow
          label={t("design.insp.borderStyle", "边框样式")}
          prop="border-style"
          value={s["border-style"] || "none"}
          options={[
            ["none", "none"],
            ["solid", "solid"],
            ["dashed", "dashed"],
            ["dotted", "dotted"],
          ]}
          onCommit={onCommitStyle}
        />
        <ColorRow
          label={t("design.insp.borderColor", "边框色")}
          prop="border-color"
          value={s["border-color"] || ""}
          onLive={onLiveStyle}
          onCommit={onCommitStyle}
        />
      </Section>

      <Section title={t("design.insp.effects", "效果")}>
        <OpacityRow value={opacity} onLive={onLiveStyle} onCommit={onCommitStyle} />
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground">{t("design.insp.shadow", "阴影")}</span>
          <div className="flex gap-0.5">
            {(
              [
                ["none", t("design.insp.shadowNone", "无")],
                ["0 1px 2px rgba(0,0,0,.08)", "S"],
                ["0 4px 12px rgba(0,0,0,.12)", "M"],
                ["0 12px 32px rgba(0,0,0,.18)", "L"],
              ] as const
            ).map(([val, lbl]) => (
              <Button
                key={lbl}
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-xs"
                onClick={() => onCommitStyle("box-shadow", val)}
              >
                {lbl}
              </Button>
            ))}
          </div>
        </div>
      </Section>
    </div>
  )
}
