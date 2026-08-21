import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Settings2 } from "lucide-react"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

export interface DesignScenario {
  id: string
  title: string
  route: string
  state: Record<string, unknown>
  viewportIds: string[]
}

export interface DesignScenarioViewport {
  width: number
  height: number
  label?: string
}

interface Manifest {
  version: number
  viewports: Record<string, DesignScenarioViewport>
  scenarios: DesignScenario[]
}

interface Props {
  artifactId: string
  onApply: (scenario: DesignScenario, viewport?: DesignScenarioViewport) => void
}

export function DesignScenariosControl({ artifactId, onApply }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [manifest, setManifest] = useState<Manifest | null>(null)
  const [selectedId, setSelectedId] = useState("default")
  const [dialogOpen, setDialogOpen] = useState(false)
  const [draft, setDraft] = useState("")
  const [busy, setBusy] = useState(false)
  const onApplyRef = useRef(onApply)

  useEffect(() => {
    onApplyRef.current = onApply
  }, [onApply])

  const applyScenario = useCallback((value: Manifest, scenario: DesignScenario) => {
    const viewportId = scenario.viewportIds[0]
    onApplyRef.current(scenario, viewportId ? value.viewports[viewportId] : undefined)
  }, [])

  useEffect(() => {
    let alive = true
    tx.call<Manifest>("get_design_scenarios_cmd", { artifactId })
      .then((value) => {
        if (!alive) return
        setManifest(value)
        const first = value.scenarios[0]
        setSelectedId(first?.id ?? "default")
        if (first) applyScenario(value, first)
      })
      .catch((e) => logger.warn("design", "DesignScenariosControl::load", "load failed", e))
    return () => {
      alive = false
    }
  }, [applyScenario, artifactId, tx])

  const scenarios = useMemo(() => manifest?.scenarios ?? [], [manifest])

  const choose = (id: string) => {
    setSelectedId(id)
    const scenario = scenarios.find((item) => item.id === id)
    if (scenario && manifest) applyScenario(manifest, scenario)
  }

  const openEditor = () => {
    const value =
      manifest ??
      ({
        version: 1,
        viewports: {},
        scenarios: [{ id: "default", title: "默认", route: "/", state: {}, viewportIds: [] }],
      } satisfies Manifest)
    setDraft(JSON.stringify(value, null, 2))
    setDialogOpen(true)
  }

  const save = async () => {
    let value: Manifest
    try {
      value = JSON.parse(draft) as Manifest
    } catch {
      toast.error(t("design.scenarios.invalid", "场景清单不是有效 JSON"))
      return
    }
    setBusy(true)
    try {
      const saved = await tx.call<Manifest>("save_design_scenarios_cmd", {
        artifactId,
        manifest: value,
      })
      setManifest(saved)
      const first = saved.scenarios[0]
      setSelectedId(first?.id ?? "default")
      if (first) applyScenario(saved, first)
      setDialogOpen(false)
      toast.success(t("design.scenarios.saved", "场景清单已保存"))
    } catch (e) {
      logger.error("design", "DesignScenariosControl::save", "save failed", e)
      toast.error(t("design.scenarios.saveFailed", "场景清单保存失败"))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <div className="flex h-6 items-center rounded-md border border-border/60 bg-background/70">
        <Select
          value={selectedId}
          disabled={scenarios.length === 0}
          onValueChange={choose}
        >
          <SelectTrigger
            aria-label={t("design.scenarios.label", "预览场景")}
            className="h-5 max-w-32 border-0 bg-transparent px-1 text-[11px]"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {scenarios.map((scenario) => (
              <SelectItem key={scenario.id} value={scenario.id}>
                {scenario.title}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button variant="ghost" size="icon" className="h-5 w-5 rounded-l-none" onClick={openEditor}>
          <Settings2 className="h-3 w-3" />
        </Button>
      </div>
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("design.scenarios.title", "预览场景清单")}</DialogTitle>
            <DialogDescription>
              {t(
                "design.scenarios.desc",
                "最多 12 个场景、4 个视口；仅保留一个活动预览，其余按需切换。route 只能是本地产物路径。",
              )}
            </DialogDescription>
          </DialogHeader>
          <Textarea
            className="min-h-[360px] font-mono text-xs"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
          />
          <DialogFooter>
            <Button variant="ghost" disabled={busy} onClick={() => setDialogOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button disabled={busy} onClick={() => void save()}>
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              {t("common.save", "保存")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
