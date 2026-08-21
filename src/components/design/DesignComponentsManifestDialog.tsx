import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, ScanSearch } from "lucide-react"
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
import { Textarea } from "@/components/ui/textarea"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

interface ComponentEntry {
  id: string
  name: string
  importPath: string
  exportName?: string
  modes: { id: string; label?: string; props: Record<string, unknown> }[]
}

interface Manifest {
  version: number
  components: ComponentEntry[]
}

interface Envelope {
  manifest: Manifest
  hash: string
  draft: boolean
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  projectId: string
}

export function DesignComponentsManifestDialog({ open, onOpenChange, projectId }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [publishedHash, setPublishedHash] = useState("")
  const [draft, setDraft] = useState("")
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (!open) return
    let alive = true
    Promise.all([
      tx.call<Envelope>("get_design_components_manifest_cmd", { projectId, draft: false }),
      tx.call<Envelope>("get_design_components_manifest_cmd", { projectId, draft: true }),
    ])
      .then(([published, currentDraft]) => {
        if (!alive) return
        setPublishedHash(published.hash)
        setDraft(JSON.stringify(currentDraft.manifest, null, 2))
      })
      .catch((e) => {
        logger.error("design", "DesignComponentsManifestDialog::load", "load failed", e)
        toast.error(t("design.components.loadFailed", "组件清单加载失败"))
      })
    return () => {
      alive = false
    }
  }, [open, projectId, tx, t])

  const parse = (): Manifest | null => {
    try {
      return JSON.parse(draft) as Manifest
    } catch {
      toast.error(t("design.components.invalid", "组件清单不是有效 JSON"))
      return null
    }
  }

  const scan = async () => {
    setBusy(true)
    try {
      const components = await tx.call<ComponentEntry[]>("scan_design_components_cmd", {
        projectId,
      })
      setDraft(JSON.stringify({ version: 1, components }, null, 2))
    } catch (e) {
      logger.error("design", "DesignComponentsManifestDialog::scan", "scan failed", e)
      toast.error(t("design.components.scanFailed", "只读扫描失败，请先绑定代码仓库"))
    } finally {
      setBusy(false)
    }
  }

  const saveDraft = async () => {
    const manifest = parse()
    if (!manifest) return
    setBusy(true)
    try {
      const saved = await tx.call<Envelope>("save_design_components_draft_cmd", {
        projectId,
        manifest,
      })
      setDraft(JSON.stringify(saved.manifest, null, 2))
      toast.success(t("design.components.draftSaved", "组件清单草稿已保存"))
    } catch (e) {
      logger.error("design", "DesignComponentsManifestDialog::save", "save failed", e)
      toast.error(t("design.components.saveFailed", "组件清单草稿保存失败"))
    } finally {
      setBusy(false)
    }
  }

  const publish = async () => {
    const manifest = parse()
    if (!manifest) return
    setBusy(true)
    try {
      const saved = await tx.call<Envelope>("publish_design_components_manifest_cmd", {
        input: { projectId, expectedPublishedHash: publishedHash, manifest },
      })
      setPublishedHash(saved.hash)
      setDraft(JSON.stringify(saved.manifest, null, 2))
      toast.success(t("design.components.published", "组件清单已发布"))
    } catch (e) {
      logger.error("design", "DesignComponentsManifestDialog::publish", "publish failed", e)
      toast.error(t("design.components.publishFailed", "发布失败，清单可能已被其他窗口更新"))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("design.components.title", "组件清单")}</DialogTitle>
          <DialogDescription>
            {t(
              "design.components.desc",
              "只读扫描绑定仓库生成候选；先保存未发布草稿，再以已发布哈希确认发布。代码始终是本地真相源。",
            )}
          </DialogDescription>
        </DialogHeader>
        <Textarea
          className="min-h-[420px] font-mono text-xs"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <DialogFooter className="sm:justify-between">
          <Button variant="outline" disabled={busy} onClick={() => void scan()}>
            <ScanSearch className="mr-1.5 h-4 w-4" />
            {t("design.components.scan", "只读扫描仓库")}
          </Button>
          <div className="flex gap-2">
            <Button variant="ghost" disabled={busy} onClick={() => void saveDraft()}>
              {t("design.components.saveDraft", "保存草稿")}
            </Button>
            <Button disabled={busy} onClick={() => void publish()}>
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              {t("design.components.publish", "确认发布")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
