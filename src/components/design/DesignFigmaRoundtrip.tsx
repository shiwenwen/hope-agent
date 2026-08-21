import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { ExternalLink, Loader2, Repeat2 } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

type Direction = "hope_to_figma" | "figma_to_hope"

interface Preview {
  id: string
  artifactId: string
  direction: Direction
  toolName: string
  arguments: Record<string, unknown>
  localHash: string
  expiresAt: string
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  artifactId: string
  onImported?: () => void
}

export function DesignFigmaRoundtrip({ open, onOpenChange, artifactId, onImported }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [direction, setDirection] = useState<Direction>("hope_to_figma")
  const [server, setServer] = useState("figma")
  const [resourceId, setResourceId] = useState("")
  const [nodeId, setNodeId] = useState("")
  const [argsText, setArgsText] = useState("{}")
  const [preview, setPreview] = useState<Preview | null>(null)
  const [busy, setBusy] = useState(false)
  const [remoteUrl, setRemoteUrl] = useState<string | null>(null)

  useEffect(() => {
    if (!open) {
      setPreview(null)
      setRemoteUrl(null)
      setArgsText("{}")
    }
  }, [open])

  const toolName = useMemo(
    () =>
      `mcp__${server.trim()}__${direction === "hope_to_figma" ? "generate_figma_design" : "get_design_context"}`,
    [server, direction],
  )

  const prepare = async () => {
    let args: Record<string, unknown>
    try {
      const parsed: unknown = JSON.parse(argsText)
      if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") throw new Error("object")
      args = parsed as Record<string, unknown>
    } catch {
      toast.error(t("design.figmaRoundtrip.invalidArgs", "MCP 参数必须是 JSON 对象"))
      return
    }
    setBusy(true)
    try {
      const next = await tx.call<Preview>("preview_figma_roundtrip_cmd", {
        input: {
          artifactId,
          direction,
          toolName,
          arguments: args,
          resourceId: resourceId.trim() || undefined,
          nodeId: nodeId.trim() || undefined,
        },
      })
      setPreview(next)
    } catch (e) {
      logger.error("design", "DesignFigmaRoundtrip::preview", "preview failed", e)
      toast.error(t("design.figmaRoundtrip.previewFailed", "Figma 往返预览失败"))
    } finally {
      setBusy(false)
    }
  }

  const commit = async () => {
    if (!preview) return
    setBusy(true)
    try {
      const result = await tx.call<{ link: { remoteUrl?: string } }>("commit_figma_roundtrip_cmd", {
        input: { previewId: preview.id, expectedLocalHash: preview.localHash },
      })
      setRemoteUrl(result.link.remoteUrl ?? null)
      setPreview(null)
      onImported?.()
      toast.success(t("design.figmaRoundtrip.done", "Figma 往返已完成并记录链接"))
    } catch (e) {
      logger.error("design", "DesignFigmaRoundtrip::commit", "commit failed", e)
      toast.error(t("design.figmaRoundtrip.commitFailed", "Figma 往返提交失败"))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Repeat2 className="h-4 w-4" />
            {t("design.figmaRoundtrip.title", "Figma 安全往返")}
          </DialogTitle>
          <DialogDescription>
            {t(
              "design.figmaRoundtrip.desc",
              "通过已登录的 Figma MCP 读取或写入；Hope 不保存 OAuth 凭据。外部写入必须先预览，再逐次确认。",
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-1.5">
              <Label>{t("design.figmaRoundtrip.direction", "方向")}</Label>
              <Select
                value={direction}
                disabled={!!preview}
                onValueChange={(value) => {
                  setDirection(value as Direction)
                  setPreview(null)
                }}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="hope_to_figma">
                  {t("design.figmaRoundtrip.toFigma", "Hope → Figma")}
                  </SelectItem>
                  <SelectItem value="figma_to_hope">
                  {t("design.figmaRoundtrip.toHope", "Figma → Hope 新版本")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="figma-mcp-server">
                {t("design.figmaRoundtrip.server", "MCP 服务器名")}
              </Label>
              <Input
                id="figma-mcp-server"
                value={server}
                disabled={!!preview}
                onChange={(event) => setServer(event.target.value)}
              />
            </div>
          </div>
          {direction === "figma_to_hope" && (
            <div className="grid grid-cols-2 gap-2">
              <Input
                value={resourceId}
                disabled={!!preview}
                onChange={(event) => setResourceId(event.target.value)}
                placeholder={t("design.figmaRoundtrip.fileKey", "Figma file key")}
              />
              <Input
                value={nodeId}
                disabled={!!preview}
                onChange={(event) => setNodeId(event.target.value)}
                placeholder={t("design.figmaRoundtrip.nodeId", "node id（可选）")}
              />
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="figma-mcp-args">
              {t("design.figmaRoundtrip.arguments", "MCP 参数（JSON）")}
            </Label>
            <Textarea
              id="figma-mcp-args"
              className="min-h-28 font-mono text-xs"
              value={argsText}
              disabled={!!preview}
              onChange={(event) => setArgsText(event.target.value)}
            />
            <p className="text-[11px] text-muted-foreground">{toolName}</p>
          </div>
          {preview && (
            <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-2 text-xs">
              <p className="font-medium">
                {t("design.figmaRoundtrip.confirmTitle", "请确认本次外部操作")}
              </p>
              <p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">
                {preview.toolName} · {preview.localHash.slice(0, 16)}…
              </p>
            </div>
          )}
          {remoteUrl && (
            <Button
              variant="outline"
              className="w-full"
              onClick={() => window.open(remoteUrl, "_blank", "noopener,noreferrer")}
            >
              <ExternalLink className="mr-1.5 h-4 w-4" />
              {t("design.figmaRoundtrip.open", "打开 Figma")}
            </Button>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" disabled={busy} onClick={() => onOpenChange(false)}>
            {t("common.close", "关闭")}
          </Button>
          {preview ? (
            <Button disabled={busy} onClick={() => void commit()}>
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              {t("design.figmaRoundtrip.confirm", "确认并提交一次")}
            </Button>
          ) : (
            <Button disabled={busy || !server.trim()} onClick={() => void prepare()}>
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              {t("design.figmaRoundtrip.preview", "生成操作预览")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
