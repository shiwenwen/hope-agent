import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Loader2, Check } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

interface CanvasConfig {
  enabled: boolean
  autoShow: boolean
  defaultContentType: string
  maxProjects: number
  maxVersionsPerProject: number
  panelWidth: number
}

export default function CanvasSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<CanvasConfig>({
    enabled: true,
    autoShow: true,
    defaultContentType: "html",
    maxProjects: 100,
    maxVersionsPerProject: 50,
    panelWidth: 480,
  })
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    invoke<CanvasConfig>("get_canvas_config")
      .then((c) => {
        setConfig(c)
        setSavedSnapshot(JSON.stringify(c))
      })
      .catch(() => {})
  }, [])

  const handleSave = async () => {
    setSaving(true)
    try {
      await invoke("save_canvas_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch {
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        {/* Description */}
        <p className="text-xs text-muted-foreground">{t("settings.canvasDesc")}</p>

        {/* Enable toggle */}
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <span className="text-sm font-medium">{t("settings.canvasEnabled")}</span>
            </div>
            <Switch
              checked={config.enabled}
              onCheckedChange={(v) => setConfig((c) => ({ ...c, enabled: v }))}
            />
          </div>

          {/* Auto-show preview */}
          <div className="flex items-center justify-between">
            <div>
              <span className="text-sm font-medium">{t("settings.canvasAutoShow")}</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("settings.canvasAutoShowDesc")}
              </p>
            </div>
            <Switch
              checked={config.autoShow}
              onCheckedChange={(v) => setConfig((c) => ({ ...c, autoShow: v }))}
            />
          </div>
        </div>

        {/* Content Settings */}
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.canvasDefaultType")}
          </h3>
          <Select
            value={config.defaultContentType}
            onValueChange={(v) => setConfig((c) => ({ ...c, defaultContentType: v }))}
          >
            <SelectTrigger className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="html">HTML</SelectItem>
              <SelectItem value="markdown">Markdown</SelectItem>
              <SelectItem value="code">Code</SelectItem>
              <SelectItem value="svg">SVG</SelectItem>
              <SelectItem value="mermaid">Mermaid</SelectItem>
              <SelectItem value="chart">Chart</SelectItem>
              <SelectItem value="slides">Slides</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Limits */}
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.canvasMaxProjects")}
          </h3>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.canvasMaxProjects")}</span>
              <Input
                type="number"
                className="w-full"
                min={1}
                max={1000}
                value={config.maxProjects}
                onChange={(e) =>
                  setConfig((c) => ({ ...c, maxProjects: parseInt(e.target.value) || 100 }))
                }
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.canvasMaxVersions")}</span>
              <Input
                type="number"
                className="w-full"
                min={1}
                max={500}
                value={config.maxVersionsPerProject}
                onChange={(e) =>
                  setConfig((c) => ({
                    ...c,
                    maxVersionsPerProject: parseInt(e.target.value) || 50,
                  }))
                }
              />
            </div>
          </div>
        </div>

      </div>
      </div>

      {/* Save — fixed bottom */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
        <Button
          onClick={handleSave}
          disabled={saving || !isDirty}
          size="sm"
          className={
            saveStatus === "saved"
              ? "bg-green-500/10 text-green-600 hover:bg-green-500/10"
              : saveStatus === "failed"
                ? "bg-destructive/10 text-destructive hover:bg-destructive/10"
                : ""
          }
        >
          {saving ? (
            <>
              <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
              {t("common.saving")}
            </>
          ) : saveStatus === "saved" ? (
            <>
              <Check className="h-3.5 w-3.5 mr-1.5" />
              {t("common.saved")}
            </>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
    </div>
  )
}
