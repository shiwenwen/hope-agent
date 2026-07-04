import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Loader2, Check } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import type { DesignConfig } from "@/types/design"

const DEFAULTS: DesignConfig = {
  enabled: true,
  autoShow: true,
  autoCritique: false,
  maxVersionsPerArtifact: 50,
  panelWidth: 480,
  selfCheck: true,
}

export default function DesignSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<DesignConfig>(DEFAULTS)
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    getTransport()
      .call<DesignConfig>("get_design_config_cmd")
      .then((c) => {
        setConfig({ ...DEFAULTS, ...c })
        setSavedSnapshot(JSON.stringify({ ...DEFAULTS, ...c }))
      })
      .catch(() => {})
  }, [])

  const handleSave = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_design_config_cmd", { config })
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

  const Toggle = ({
    label,
    desc,
    value,
    onChange,
  }: {
    label: string
    desc?: string
    value: boolean
    onChange: (v: boolean) => void
  }) => (
    <div className="flex items-center justify-between">
      <div>
        <span className="text-sm font-medium">{label}</span>
        {desc && <p className="mt-0.5 text-xs text-muted-foreground">{desc}</p>}
      </div>
      <Switch checked={value} onCheckedChange={onChange} />
    </div>
  )

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-6">
          <p className="text-xs text-muted-foreground">
            {t("design.settings.desc", "设计空间：生成、微调、导出可交付的设计产物。")}
          </p>

          <div className="space-y-4">
            <Toggle
              label={t("design.settings.enabled", "启用设计空间")}
              value={config.enabled}
              onChange={(v) => setConfig((c) => ({ ...c, enabled: v }))}
            />
            <Toggle
              label={t("design.settings.autoShow", "生成后自动聚焦预览")}
              value={config.autoShow}
              onChange={(v) => setConfig((c) => ({ ...c, autoShow: v }))}
            />
            <Toggle
              label={t("design.settings.autoCritique", "定稿前自动跑质量评审")}
              desc={t(
                "design.settings.autoCritiqueDesc",
                "对产物做 5 维质量评审（会产生一次模型调用成本）。",
              )}
              value={config.autoCritique}
              onChange={(v) => setConfig((c) => ({ ...c, autoCritique: v }))}
            />
            <Toggle
              label={t("design.settings.selfCheck", "反 AI-slop 自查")}
              value={config.selfCheck}
              onChange={(v) => setConfig((c) => ({ ...c, selfCheck: v }))}
            />
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("design.settings.maxVersions", "单产物保留版本数")}
              </span>
              <DeferredNumberInput
                className="w-full"
                min={1}
                max={500}
                value={config.maxVersionsPerArtifact}
                onValueCommit={(value) =>
                  setConfig((c) => ({ ...c, maxVersionsPerArtifact: value }))
                }
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("design.settings.panelWidth", "面板默认宽度")}
              </span>
              <DeferredNumberInput
                className="w-full"
                min={320}
                max={960}
                value={config.panelWidth}
                onValueCommit={(value) => setConfig((c) => ({ ...c, panelWidth: value }))}
              />
            </div>
          </div>
        </div>
      </div>

      <div className="flex items-center justify-end gap-2 border-t p-4">
        <Button onClick={handleSave} disabled={!isDirty || saving} className="min-w-24">
          {saving ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : saveStatus === "saved" ? (
            <>
              <Check className="mr-1.5 h-4 w-4 text-green-500" />
              {t("common.saved", "已保存")}
            </>
          ) : saveStatus === "failed" ? (
            <span className="text-destructive">{t("common.saveFailed", "保存失败")}</span>
          ) : (
            t("common.save", "保存")
          )}
        </Button>
      </div>
    </div>
  )
}
