/**
 * 从 Figma 文件导入设计系统（P3 工程轴 B）。
 *
 * owner 平面专属：用户粘贴 Figma 文件 URL/key + 个人访问令牌，后端拉已发布 styles（颜色 /
 * 文字 / 阴影）或回退采样文档填充色 → LLM 蒸馏成 9 段设计契约 + tokens。**令牌按次传、不落盘**，
 * 也不进模型面（凭据安全）。走 owner 命令 `import_figma_system_cmd`。
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { DesignSystemMeta } from "@/types/design"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImported: (systemId: string) => void
}

export function DesignFigmaImport({ open, onOpenChange, onImported }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [url, setUrl] = useState("")
  const [token, setToken] = useState("")
  const [name, setName] = useState("")
  const [importing, setImporting] = useState(false)

  // 关闭即清空（取消 / Esc / 点遮罩都算）——凭据不在内存里滞留（review #5）。
  useEffect(() => {
    if (!open) {
      setUrl("")
      setToken("")
      setName("")
    }
  }, [open])

  const run = async () => {
    if (!url.trim() || !token.trim()) return
    setImporting(true)
    try {
      const meta = await tx.call<DesignSystemMeta>("import_figma_system_cmd", {
        url: url.trim(),
        token: token.trim(),
        name: name.trim() || undefined,
      })
      if (meta) onImported(meta.id)
      toast.success(t("design.figma.ok", "已从 Figma 导入设计系统"))
      onOpenChange(false)
      setUrl("")
      setToken("")
      setName("")
    } catch (e) {
      logger.error("design", "DesignFigmaImport::run", "figma import failed", e)
      toast.error(t("design.figma.err", "Figma 导入失败，请检查 URL 与令牌"))
    } finally {
      setImporting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("design.figma.title", "从 Figma 导入")}</DialogTitle>
          <DialogDescription>
            {t(
              "design.figma.desc",
              "拉取 Figma 文件已发布的颜色 / 文字 / 阴影样式，蒸馏成品牌设计系统。令牌仅本次使用、不会保存。",
            )}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-1">
          <div className="space-y-1.5">
            <Label htmlFor="figma-url">{t("design.figma.url", "Figma 文件 URL 或 key")}</Label>
            <Input
              id="figma-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://www.figma.com/design/AbC123/…"
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="figma-token">{t("design.figma.token", "个人访问令牌")}</Label>
            <Input
              id="figma-token"
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="figd_…"
              autoComplete="off"
            />
            <p className="text-[11px] text-muted-foreground">
              {t(
                "design.figma.tokenHint",
                "在 Figma → Settings → Security → Personal access tokens 生成（需 file_read 权限）。",
              )}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="figma-name">{t("design.figma.name", "系统名称（可选）")}</Label>
            <Input
              id="figma-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("design.figma.namePlaceholder", "Figma 设计系统")}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={importing}>
            {t("common.cancel", "取消")}
          </Button>
          <Button onClick={() => void run()} disabled={importing || !url.trim() || !token.trim()}>
            {importing && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
            {t("design.figma.import", "导入")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
