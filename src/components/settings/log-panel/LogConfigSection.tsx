import { useTranslation } from "react-i18next"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { LEVELS } from "./constants"
import type { LogConfig } from "../types"

interface LogConfigSectionProps {
  config: LogConfig
  onSaveConfig: (config: LogConfig) => void
}

export default function LogConfigSection({ config, onSaveConfig }: LogConfigSectionProps) {
  const { t } = useTranslation()

  return (
    <div className="rounded-lg border border-border p-4 space-y-3 bg-secondary/20">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium">{t("settings.logsEnabled")}</p>
          <p className="text-xs text-muted-foreground">{t("settings.logsEnabledDesc")}</p>
        </div>
        <Switch
          checked={config.enabled}
          onCheckedChange={(checked) => onSaveConfig({ ...config, enabled: checked })}
        />
      </div>
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium">{t("settings.logsFileEnabled")}</p>
          <p className="text-xs text-muted-foreground">{t("settings.logsFileEnabledDesc")}</p>
        </div>
        <Switch
          checked={config.fileEnabled}
          onCheckedChange={(checked) => onSaveConfig({ ...config, fileEnabled: checked })}
        />
      </div>
      <div className="grid grid-cols-4 gap-3">
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsLevel")}</label>
          <Select
            value={config.level}
            onValueChange={(value) => onSaveConfig({ ...config, level: value })}
          >
            <SelectTrigger className="mt-1 h-8 w-full text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LEVELS.map((l) => (
                <SelectItem key={l} value={l}>
                  {l}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsMaxAge")}</label>
          <DeferredNumberInput
            value={config.maxAgeDays}
            onValueCommit={(value) => onSaveConfig({ ...config, maxAgeDays: value })}
            className="mt-1 h-8 text-sm"
            min={1}
            max={365}
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsMaxSize")}</label>
          <DeferredNumberInput
            value={config.maxSizeMb}
            onValueCommit={(value) => onSaveConfig({ ...config, maxSizeMb: value })}
            className="mt-1 h-8 text-sm"
            min={10}
            max={1000}
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsFileMaxSize")}</label>
          <DeferredNumberInput
            value={config.fileMaxSizeMb}
            onValueCommit={(value) => onSaveConfig({ ...config, fileMaxSizeMb: value })}
            className="mt-1 h-8 text-sm"
            min={1}
            max={100}
          />
        </div>
      </div>
    </div>
  )
}
