/**
 * 设计变量可视化编辑器（P2 护城河：两家云端竞品都没有的逐 token 可视化编辑）。
 *
 * 加载一个设计系统的 tokens（`--ds-*`）→ 按前缀分组、逐 token 编辑：颜色值给取色器 + hex，
 * 其余给文本框；可**可视化 ↔ 源码**切换（源码 = `--key: value` 逐行）。保存走 owner 命令
 * `save_design_system_cmd`：user/extracted 就地更新；内置只读 → 存为「我的」新副本（fork）。
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Code2, Eye, Check } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { DesignSystemFull, DesignSystemMeta } from "@/types/design"

/** 值是否像颜色（给取色器）。 */
function isColorValue(v: string): boolean {
  const s = v.trim().toLowerCase()
  return (
    /^#[0-9a-f]{3,8}$/.test(s) ||
    /^(rgb|hsl)a?\(/.test(s) ||
    /^(transparent|currentcolor|white|black|red|green|blue|gray|grey)$/.test(s)
  )
}

/** 任意颜色 → `#rrggbb`（`<input type=color>` 可接受）；解析失败回退黑。 */
function toHex(v: string): string {
  const s = (v || "").trim()
  if (/^#[0-9a-fA-F]{6}$/.test(s)) return s.toLowerCase()
  if (/^#[0-9a-fA-F]{3}$/.test(s)) {
    const [r, g, b] = [s[1], s[2], s[3]]
    return `#${r}${r}${g}${g}${b}${b}`.toLowerCase()
  }
  try {
    const ctx = document.createElement("canvas").getContext("2d")
    if (ctx) {
      ctx.fillStyle = "#000000"
      ctx.fillStyle = s
      const r = ctx.fillStyle
      if (/^#[0-9a-fA-F]{6}$/.test(r)) return r.toLowerCase()
      const m = r.match(/rgba?\(([^)]+)\)/)
      if (m) {
        const [rr, gg, bb] = m[1].split(",").map((x) => parseInt(x.trim(), 10))
        const h = (n: number) => Math.max(0, Math.min(255, n || 0)).toString(16).padStart(2, "0")
        return `#${h(rr)}${h(gg)}${h(bb)}`
      }
    }
  } catch {
    /* ignore */
  }
  return "#000000"
}

/** `--ds-color-primary` → 分组 `color`。 */
function groupOf(key: string): string {
  return key.replace(/^--ds-/, "").split("-")[0] || "misc"
}

/** tokens → 源码文本（每行 `--key: value`）。 */
function toSource(tokens: Record<string, string>): string {
  return Object.entries(tokens)
    .map(([k, v]) => `${k}: ${v}`)
    .join("\n")
}

/** 源码文本 → tokens（宽松：忽略空行/无冒号行，末尾分号去掉）。 */
function parseSource(text: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const line of text.split("\n")) {
    const t = line.trim().replace(/;$/, "")
    if (!t) continue
    const i = t.indexOf(":")
    if (i <= 0) continue
    const k = t.slice(0, i).trim()
    const v = t.slice(i + 1).trim()
    if (k) out[k] = v
  }
  return out
}

interface Props {
  system: DesignSystemMeta | null
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 保存成功（回传新/更新的系统 id 供上层刷新 / 选中）。 */
  onSaved: (systemId: string) => void
}

export function DesignTokenEditor({ system, open, onOpenChange, onSaved }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [full, setFull] = useState<DesignSystemFull | null>(null)
  const [tokens, setTokens] = useState<Record<string, string>>({})
  const [mode, setMode] = useState<"visual" | "source">("visual")
  const [sourceText, setSourceText] = useState("")
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  const isBuiltin = system?.source === "builtin"

  // 打开时加载完整系统（tokens + system_md）。
  useEffect(() => {
    if (!open || !system) return
    setLoading(true)
    setMode("visual")
    tx.call<DesignSystemFull>("get_design_system_cmd", { id: system.id })
      .then((f) => {
        setFull(f)
        setTokens({ ...f.tokens })
      })
      .catch((e) => {
        logger.error("design", "DesignTokenEditor::load", "load system failed", e)
        toast.error(t("design.token.loadErr", "加载设计变量失败"))
        onOpenChange(false)
      })
      .finally(() => setLoading(false))
  }, [open, system, tx, t, onOpenChange])

  // 切到源码：把当前 tokens 序列化；切回可视化：解析源码。
  const switchMode = (next: "visual" | "source") => {
    if (next === mode) return
    if (next === "source") setSourceText(toSource(tokens))
    else setTokens(parseSource(sourceText))
    setMode(next)
  }

  const setToken = (key: string, value: string) => setTokens((m) => ({ ...m, [key]: value }))

  const save = async () => {
    if (!full || !system) return
    const finalTokens = mode === "source" ? parseSource(sourceText) : tokens
    setSaving(true)
    try {
      const res = await tx.call<DesignSystemMeta>("save_design_system_cmd", {
        input: {
          // 内置只读 → fork 为新「我的」系统（不传 id）；否则就地更新。
          id: isBuiltin ? undefined : system.id,
          name: isBuiltin ? `${system.name} ${t("design.token.copySuffix", "副本")}` : system.name,
          summary: system.summary,
          systemMd: full.systemMd,
          tokens: finalTokens,
          source: isBuiltin ? "user" : system.source,
        },
      })
      toast.success(t("design.token.saved", "设计变量已保存"))
      onSaved(res.id)
      onOpenChange(false)
    } catch (e) {
      logger.error("design", "DesignTokenEditor::save", "save failed", e)
      toast.error(t("design.token.saveErr", "保存失败"))
    } finally {
      setSaving(false)
    }
  }

  // 可视化分组。
  const groups = Object.keys(tokens)
    .sort()
    .reduce<Record<string, string[]>>((acc, k) => {
      const g = groupOf(k)
      ;(acc[g] = acc[g] || []).push(k)
      return acc
    }, {})

  return (
    <Dialog open={open} onOpenChange={(o) => !saving && onOpenChange(o)}>
      <DialogContent className="max-w-lg gap-0 overflow-hidden p-0">
        <DialogHeader className="flex-row items-center justify-between gap-2 border-b px-4 py-3">
          <DialogTitle className="flex items-center gap-2 text-sm">
            {t("design.token.title", "编辑设计变量")}
            {system && <span className="text-xs font-normal text-muted-foreground">{system.name}</span>}
          </DialogTitle>
          <div className="flex items-center gap-0.5 rounded-md border p-0.5">
            <Button
              variant={mode === "visual" ? "secondary" : "ghost"}
              size="sm"
              className="h-6 gap-1 px-2 text-xs"
              onClick={() => switchMode("visual")}
            >
              <Eye className="h-3.5 w-3.5" />
              {t("design.token.visual", "可视化")}
            </Button>
            <Button
              variant={mode === "source" ? "secondary" : "ghost"}
              size="sm"
              className="h-6 gap-1 px-2 text-xs"
              onClick={() => switchMode("source")}
            >
              <Code2 className="h-3.5 w-3.5" />
              {t("design.token.source", "源码")}
            </Button>
          </div>
        </DialogHeader>

        <div className="max-h-[56vh] overflow-y-auto p-3">
          {loading ? (
            <div className="flex items-center justify-center py-12 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : mode === "source" ? (
            <Textarea
              value={sourceText}
              onChange={(e) => setSourceText(e.target.value)}
              rows={16}
              spellCheck={false}
              className="font-mono text-xs"
            />
          ) : Object.keys(groups).length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {t("design.token.empty", "该设计系统没有可编辑的变量")}
            </p>
          ) : (
            <div className="space-y-3">
              {Object.entries(groups).map(([group, keys]) => (
                <div key={group}>
                  <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                    {group}
                  </div>
                  <div className="space-y-1">
                    {keys.map((k) => {
                      const v = tokens[k] ?? ""
                      const color = isColorValue(v)
                      return (
                        <div key={k} className="flex items-center gap-2">
                          <span
                            className="w-40 shrink-0 truncate font-mono text-[11px] text-muted-foreground"
                            title={k}
                          >
                            {k.replace(/^--ds-/, "")}
                          </span>
                          {color && (
                            <input
                              type="color"
                              value={toHex(v)}
                              onChange={(e) => setToken(k, e.target.value)}
                              className="h-6 w-7 shrink-0 cursor-pointer rounded border bg-transparent p-0"
                            />
                          )}
                          <Input
                            value={v}
                            onChange={(e) => setToken(k, e.target.value)}
                            className="h-7 flex-1 font-mono text-xs"
                          />
                        </div>
                      )
                    })}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <DialogFooter className="border-t px-4 py-2.5">
          {isBuiltin && (
            <span className="mr-auto self-center text-[11px] text-muted-foreground">
              {t("design.token.forkHint", "内置系统只读，保存将创建你的副本")}
            </span>
          )}
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={saving}>
            {t("common.cancel", "取消")}
          </Button>
          <Button onClick={() => void save()} disabled={saving || loading}>
            {saving ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Check className="mr-2 h-4 w-4" />
            )}
            {isBuiltin ? t("design.token.saveCopy", "存为副本") : t("common.save", "保存")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
