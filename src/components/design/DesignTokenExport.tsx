/**
 * 多平台 Token 导出对话框（P3 工程轴 A）。
 *
 * 把一个设计系统的 `--ds-*` tokens 导出成开发者可直接落地的六种格式（CSS / SCSS / TS /
 * Swift / Android XML / DTCG JSON）。全部生成在后端纯函数 `token_export`（确定性、无网络），
 * 前端只做展示 + 复制 + 下载。owner 平面：走 `export_design_tokens_cmd`。
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Copy, Check, Download } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { downloadBlob, safeFilename } from "@/lib/designExport"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { DesignSystemMeta, TokenExport } from "@/types/design"

interface Props {
  system: DesignSystemMeta | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function DesignTokenExport({ system, open, onOpenChange }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [loading, setLoading] = useState(false)
  const [exports, setExports] = useState<TokenExport[]>([])
  const [copied, setCopied] = useState<string | null>(null)

  useEffect(() => {
    if (!open || !system) return
    let alive = true
    setLoading(true)
    setExports([])
    void (async () => {
      try {
        const res = await tx.call<TokenExport[]>("export_design_tokens_cmd", {
          systemId: system.id,
        })
        if (alive) setExports(res ?? [])
      } catch (e) {
        logger.error("design", "DesignTokenExport::load", "export tokens failed", e)
        if (alive) toast.error(t("design.export.loadErr", "生成导出失败"))
      } finally {
        if (alive) setLoading(false)
      }
    })()
    return () => {
      alive = false
    }
  }, [open, system, tx, t])

  const copy = async (e: TokenExport) => {
    try {
      await navigator.clipboard.writeText(e.content)
      setCopied(e.format)
      window.setTimeout(() => setCopied((c) => (c === e.format ? null : c)), 1600)
    } catch (err) {
      logger.error("design", "DesignTokenExport::copy", "clipboard failed", err)
      toast.error(t("design.export.copyErr", "复制失败"))
    }
  }

  const download = (e: TokenExport) => {
    const prefix = system ? `${safeFilename(system.name)}-` : ""
    downloadBlob(new Blob([e.content], { type: "text/plain;charset=utf-8" }), `${prefix}${e.filename}`)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[82vh] max-w-3xl flex-col gap-3">
        <DialogHeader>
          <DialogTitle>
            {t("design.export.title", "导出 Token")}
            {system ? ` · ${system.name}` : ""}
          </DialogTitle>
          <DialogDescription>
            {t(
              "design.export.desc",
              "把设计变量导出为各平台可直接落地的开发者代码，供工程侧接入。",
            )}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex items-center justify-center py-16 text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : exports.length === 0 ? (
          <p className="py-12 text-center text-sm text-muted-foreground">
            {t("design.export.empty", "该设计系统没有可导出的变量")}
          </p>
        ) : (
          <Tabs defaultValue={exports[0]?.format} className="flex min-h-0 flex-1 flex-col">
            <TabsList className="flex-wrap justify-start">
              {exports.map((e) => (
                <TabsTrigger key={e.format} value={e.format} className="text-xs">
                  {e.label}
                </TabsTrigger>
              ))}
            </TabsList>
            {exports.map((e) => (
              <TabsContent
                key={e.format}
                value={e.format}
                className="mt-2 flex min-h-0 flex-1 flex-col gap-2"
              >
                <div className="flex items-center gap-2">
                  <code className="flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-[11px] text-muted-foreground">
                    {e.filename}
                  </code>
                  <Button variant="outline" size="sm" className="h-7 gap-1.5" onClick={() => copy(e)}>
                    {copied === e.format ? (
                      <Check className="h-3.5 w-3.5 text-green-500" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    {t("design.export.copy", "复制")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1.5"
                    onClick={() => download(e)}
                  >
                    <Download className="h-3.5 w-3.5" />
                    {t("design.export.download", "下载")}
                  </Button>
                </div>
                <pre className="min-h-0 flex-1 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
                  {e.content}
                </pre>
              </TabsContent>
            ))}
          </Tabs>
        )}
      </DialogContent>
    </Dialog>
  )
}
