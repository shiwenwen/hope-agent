import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Info } from "lucide-react"
import { cn } from "@/lib/utils"
import type { AgentConfig } from "../../types"
import type { SessionMode } from "@/types/chat"

interface ApprovalTabProps {
  config: AgentConfig
  updateConfig: (patch: Partial<AgentConfig>) => void
}

/**
 * Tools the user can opt into approving in Default mode. Mirrors the v2 plan:
 * - Hardcoded "always-approve": write / edit / apply_patch + exec command
 *   patterns (edit/dangerous lists). Not in this list — UI never shows them.
 * - "Always-allow" (基础核心 read/grep/find/memory/cron/canvas/etc.) Not here.
 * - The remaining 17 tools below are the user-curated extras. MCP dynamic
 *   tools are controlled per-server in the MCP settings panel.
 */
const APPROVAL_OPTIN_GROUPS: ReadonlyArray<{
  groupKey: string
  tools: ReadonlyArray<string>
}> = [
  {
    groupKey: "shell",
    tools: ["process"],
  },
  {
    groupKey: "browser",
    tools: ["browser"],
  },
  {
    groupKey: "settings",
    tools: ["update_settings", "restore_settings_backup"],
  },
  {
    groupKey: "outbound",
    tools: ["send_attachment", "sessions_send"],
  },
  {
    groupKey: "paid",
    tools: ["image_generate"],
  },
  {
    groupKey: "spawn",
    tools: ["acp_spawn"],
  },
  {
    groupKey: "network",
    tools: ["web_fetch", "web_search"],
  },
  {
    groupKey: "crossSession",
    tools: [
      "peek_sessions",
      "sessions_list",
      "sessions_history",
      "session_status",
      "agents_list",
    ],
  },
  {
    groupKey: "settingsRead",
    tools: ["get_settings", "list_settings_backups"],
  },
]

const MODE_OPTIONS: ReadonlyArray<{ value: SessionMode | "inherit" }> = [
  { value: "inherit" },
  { value: "default" },
  { value: "smart" },
  { value: "yolo" },
]

export default function ApprovalTab({ config, updateConfig }: ApprovalTabProps) {
  const { t } = useTranslation()

  const enabled = config.capabilities.enableCustomToolApproval ?? false
  const customList = config.capabilities.customApprovalTools ?? []
  const defaultMode =
    (config.capabilities.defaultSessionPermissionMode as SessionMode | undefined) ?? null

  const updateCaps = (patch: Partial<AgentConfig["capabilities"]>) =>
    updateConfig({ capabilities: { ...config.capabilities, ...patch } })

  const setEnabled = (v: boolean) => updateCaps({ enableCustomToolApproval: v })

  const toggleTool = (name: string, on: boolean) => {
    const next = on
      ? Array.from(new Set([...customList, name]))
      : customList.filter((n) => n !== name)
    updateCaps({ customApprovalTools: next })
  }

  const setDefaultMode = (mode: SessionMode | "inherit") => {
    updateCaps({
      defaultSessionPermissionMode: mode === "inherit" ? null : mode,
    })
  }

  const selectedMode: SessionMode | "inherit" = defaultMode ?? "inherit"

  return (
    <div className="space-y-6">
      {/* Default session permission mode */}
      <section>
        <h3 className="text-sm font-medium text-foreground mb-1">
          {t("settings.agentApproval.defaultModeTitle")}
        </h3>
        <p className="text-xs text-muted-foreground mb-3">
          {t("settings.agentApproval.defaultModeDesc")}
        </p>
        <div className="grid gap-2 sm:grid-cols-2" role="radiogroup">
          {MODE_OPTIONS.map((opt) => {
            const isActive = selectedMode === opt.value
            return (
              <button
                key={opt.value}
                type="button"
                role="radio"
                aria-checked={isActive}
                onClick={() => setDefaultMode(opt.value)}
                className={cn(
                  "flex items-start gap-2 rounded-lg border p-3 text-left transition-all",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                  isActive
                    ? "border-primary bg-primary/5"
                    : "border-border/50 hover:border-border",
                )}
              >
                <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border border-current">
                  <span
                    className={cn(
                      "h-2 w-2 rounded-full bg-primary transition-opacity",
                      isActive ? "opacity-100" : "opacity-0",
                    )}
                  />
                </span>
                <span className="flex-1">
                  <span className="block text-xs font-medium">
                    {t(`settings.agentApproval.modes.${opt.value}.label`)}
                  </span>
                  <span className="block text-[11px] text-muted-foreground mt-0.5">
                    {t(`settings.agentApproval.modes.${opt.value}.desc`)}
                  </span>
                </span>
              </button>
            )
          })}
        </div>
      </section>

      <div className="border-t border-border/50" />

      {/* Custom Tool Approval master switch */}
      <section>
        <div className="flex items-start justify-between gap-3 mb-3">
          <div>
            <h3 className="text-sm font-medium text-foreground">
              {t("settings.agentApproval.customApprovalTitle")}
            </h3>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("settings.agentApproval.customApprovalDesc")}
            </p>
          </div>
          <Switch checked={enabled} onCheckedChange={setEnabled} />
        </div>

        <div
          className={cn(
            "rounded-md border border-amber-200/40 bg-amber-50/40 dark:bg-amber-950/10 p-2.5 mb-4 text-[11px] text-amber-700 dark:text-amber-400 flex items-start gap-2",
          )}
        >
          <Info className="h-3.5 w-3.5 mt-0.5 shrink-0" />
          <span>{t("settings.agentApproval.customApprovalHint")}</span>
        </div>

        <div
          className={cn(
            "space-y-4 transition-opacity",
            !enabled && "opacity-40 pointer-events-none",
          )}
          aria-disabled={!enabled}
        >
          {APPROVAL_OPTIN_GROUPS.map(({ groupKey, tools }) => (
            <div key={groupKey}>
              <div className="text-[11px] font-medium text-muted-foreground/70 mb-1.5 uppercase tracking-wide">
                {t(`settings.agentApproval.groups.${groupKey}`)}
              </div>
              <div className="rounded-lg border border-border/50 overflow-hidden">
                {tools.map((toolName, idx) => (
                  <div
                    key={toolName}
                    className={cn(
                      "flex items-center justify-between px-3 py-2 gap-3",
                      idx > 0 && "border-t border-border/30",
                    )}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="text-xs font-medium text-foreground">
                        {t(`settings.agentApproval.toolNames.${toolName}`, toolName)}
                      </div>
                      <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                        {t(`settings.agentApproval.toolDescs.${toolName}`, "")}
                      </div>
                    </div>
                    <Switch
                      checked={customList.includes(toolName)}
                      onCheckedChange={(v) => toggleTool(toolName, v)}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}
