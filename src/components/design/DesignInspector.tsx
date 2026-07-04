/**
 * 属性检视器（D1 可视化微调的可见半边）。
 *
 * 接收 iframe bridge 回传的选中元素，提供分区控件（文本/颜色/排版/间距/圆角）：
 * 交互时**即时预览**（回调驱动 iframe live style），交互结束**提交**回写源码。
 * 控件是纯受控组件，父层负责 preview / commit 两条通道。
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { X, AlignLeft, AlignCenter, AlignRight } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import type { DesignSelectedElement } from "@/types/design"

interface Props {
  selected: DesignSelectedElement
  onLiveStyle: (prop: string, value: string) => void
  onCommitStyle: (prop: string, value: string) => void
  onLiveText: (text: string) => void
  onCommitText: (text: string) => void
  onClose: () => void
}

/** computed `rgb(a,b,c)` / `#rrggbb` → `#rrggbb`（native color input 需要）。 */
function toHex(v: string): string {
  if (!v) return "#000000"
  if (v.startsWith("#")) return v.length === 7 ? v : "#000000"
  const m = v.match(/rgba?\(([^)]+)\)/)
  if (!m) return "#000000"
  const [r, g, b] = m[1].split(",").map((x) => parseInt(x.trim(), 10))
  const h = (n: number) => Math.max(0, Math.min(255, n || 0)).toString(16).padStart(2, "0")
  return `#${h(r)}${h(g)}${h(b)}`
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
  return (
    <label className="flex items-center justify-between gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="flex items-center gap-1.5">
        <span className="font-mono text-xs text-muted-foreground">{hex}</span>
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
  useEffect(() => setV(String(value)), [value])
  const commit = () => onCommit(prop, `${parseFloat(v) || 0}${suffix}`)
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

export default function DesignInspector({
  selected,
  onLiveStyle,
  onCommitStyle,
  onLiveText,
  onCommitText,
  onClose,
}: Props) {
  const { t } = useTranslation()
  const s = selected.styles
  const [text, setText] = useState(selected.text)
  useEffect(() => setText(selected.text), [selected.oid, selected.text])

  const align = s["text-align"] || "left"

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
          <textarea
            value={text}
            onChange={(e) => {
              setText(e.target.value)
              onLiveText(e.target.value)
            }}
            onBlur={() => onCommitText(text)}
            rows={2}
            className="w-full resize-none rounded border bg-background px-2 py-1.5 text-sm"
          />
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
        <NumberRow
          label={t("design.insp.padding", "内边距")}
          prop="padding"
          value={px(s["padding"] || "0")}
          onCommit={onCommitStyle}
        />
        <NumberRow
          label={t("design.insp.margin", "外边距")}
          prop="margin"
          value={px(s["margin"] || "0")}
          onCommit={onCommitStyle}
        />
        <NumberRow
          label={t("design.insp.radius", "圆角")}
          prop="border-radius"
          value={px(s["border-radius"] || "0")}
          onCommit={onCommitStyle}
        />
      </Section>
    </div>
  )
}
