import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
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
          <select
            value={config.level}
            onChange={(e) => onSaveConfig({ ...config, level: e.target.value })}
            className="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 text-sm"
          >
            {LEVELS.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsMaxAge")}</label>
          <Input
            type="number"
            value={config.maxAgeDays}
            onChange={(e) =>
              onSaveConfig({ ...config, maxAgeDays: parseInt(e.target.value) || 30 })
            }
            className="mt-1 h-8 text-sm"
            min={1}
            max={365}
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground">{t("settings.logsMaxSize")}</label>
          <Input
            type="number"
            value={config.maxSizeMb}
            onChange={(e) =>
              onSaveConfig({ ...config, maxSizeMb: parseInt(e.target.value) || 100 })
            }
            className="mt-1 h-8 text-sm"
            min={10}
            max={1000}
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground">
            {t("settings.logsFileMaxSize")}
          </label>
          <Input
            type="number"
            value={config.fileMaxSizeMb}
            onChange={(e) =>
              onSaveConfig({ ...config, fileMaxSizeMb: parseInt(e.target.value) || 10 })
            }
            className="mt-1 h-8 text-sm"
            min={1}
            max={100}
          />
        </div>
      </div>
    </div>
  )
}
