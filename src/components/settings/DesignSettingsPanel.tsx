import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Loader2, Check } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { DesignConfig, DesignSystemMeta } from "@/types/design"

interface ProviderOption {
  id: string
  name: string
  models: { id: string; name: string }[]
  enabled?: boolean
}

const NONE = "__none__"
const MODEL_DEFAULT = "__default__"

const DEFAULTS: DesignConfig = {
  enabled: true,
  autoShow: true,
  autoCritique: false,
  defaultSystemId: undefined,
  maxVersionsPerArtifact: 50,
  panelWidth: 480,
  selfCheck: true,
  maxExtractImageMb: 24,
  exportScale: 2,
  exportJpegQuality: 92,
  extractVisionModel: undefined,
  critiqueModel: undefined,
}

/** Optional model override picker (`providerId:modelId`); default = reuse. */
function ModelPicker({
  label,
  desc,
  value,
  providers,
  onChange,
  defaultLabel,
}: {
  label: string
  desc: string
  value: string | undefined
  providers: ProviderOption[]
  onChange: (v: string | undefined) => void
  defaultLabel: string
}) {
  return (
    <div className="space-y-1.5">
      <span className="text-sm font-medium">{label}</span>
      <p className="text-xs text-muted-foreground">{desc}</p>
      <Select
        value={value || MODEL_DEFAULT}
        onValueChange={(v) => onChange(v === MODEL_DEFAULT ? undefined : v)}
      >
        <SelectTrigger className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={MODEL_DEFAULT}>{defaultLabel}</SelectItem>
          {providers
            .filter((p) => p.enabled !== false && p.models.length > 0)
            .map((p) => (
              <SelectGroup key={p.id}>
                <SelectLabel>{p.name}</SelectLabel>
                {p.models.map((m) => (
                  <SelectItem key={`${p.id}:${m.id}`} value={`${p.id}:${m.id}`}>
                    {m.name}
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
        </SelectContent>
      </Select>
    </div>
  )
}

export default function DesignSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<DesignConfig>(DEFAULTS)
  const [systems, setSystems] = useState<DesignSystemMeta[]>([])
  const [providers, setProviders] = useState<ProviderOption[]>([])
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
    getTransport()
      .call<DesignSystemMeta[]>("list_design_systems_cmd")
      .then((list) => setSystems(list ?? []))
      .catch(() => {})
    getTransport()
      .call<ProviderOption[]>("get_providers")
      .then((list) => setProviders(list ?? []))
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

          <div className="space-y-1.5">
            <span className="text-sm font-medium">
              {t("design.settings.defaultSystem", "默认设计系统")}
            </span>
            <p className="text-xs text-muted-foreground">
              {t(
                "design.settings.defaultSystemDesc",
                "新产物在项目未指定设计系统时回退到此。",
              )}
            </p>
            <Select
              value={config.defaultSystemId || NONE}
              onValueChange={(v) =>
                setConfig((c) => ({ ...c, defaultSystemId: v === NONE ? undefined : v }))
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>{t("design.settings.systemNone", "无")}</SelectItem>
                {systems.map((s) => (
                  <SelectItem key={s.id} value={s.id}>
                    {s.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
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
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("design.settings.maxExtractImageMb", "提取图片大小上限 (MB)")}
              </span>
              <DeferredNumberInput
                className="w-full"
                min={0}
                max={512}
                value={config.maxExtractImageMb}
                onValueCommit={(value) => setConfig((c) => ({ ...c, maxExtractImageMb: value }))}
              />
              <p className="text-xs text-muted-foreground">
                {t("design.settings.maxExtractImageMbDesc", "反向提取截图时的图片上限；0 = 不限。")}
              </p>
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("design.settings.exportScale", "导出清晰度 (倍)")}
              </span>
              <DeferredNumberInput
                className="w-full"
                min={1}
                max={4}
                value={config.exportScale}
                onValueCommit={(value) => setConfig((c) => ({ ...c, exportScale: value }))}
              />
              <p className="text-xs text-muted-foreground">
                {t("design.settings.exportScaleDesc", "栅格化倍率；越大越清晰、文件越大。默认 2。")}
              </p>
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("design.settings.exportJpegQuality", "PDF 图像质量")}
              </span>
              <DeferredNumberInput
                className="w-full"
                min={40}
                max={100}
                value={config.exportJpegQuality}
                onValueCommit={(value) => setConfig((c) => ({ ...c, exportJpegQuality: value }))}
              />
              <p className="text-xs text-muted-foreground">
                {t("design.settings.exportJpegQualityDesc", "PDF 页 JPEG 压缩质量 (1–100)。默认 92。")}
              </p>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <ModelPicker
              label={t("design.settings.extractVisionModel", "提取视觉模型")}
              desc={t(
                "design.settings.extractVisionModelDesc",
                "反向提取截图专用；留空复用当前活跃模型（需支持视觉）。",
              )}
              value={config.extractVisionModel}
              providers={providers}
              onChange={(v) => setConfig((c) => ({ ...c, extractVisionModel: v }))}
              defaultLabel={t("design.settings.modelActive", "复用活跃模型")}
            />
            <ModelPicker
              label={t("design.settings.critiqueModel", "质量评审模型")}
              desc={t(
                "design.settings.critiqueModelDesc",
                "5 维质量评审专用；留空复用默认分析模型。",
              )}
              value={config.critiqueModel}
              providers={providers}
              onChange={(v) => setConfig((c) => ({ ...c, critiqueModel: v }))}
              defaultLabel={t("design.settings.modelDefault", "复用默认分析模型")}
            />
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
