import { useEffect, useState, useEffectEvent } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { ChevronRight, Copy, Loader2, Plus, Users2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  TEAM_EVENT_CHANNEL,
  TEAM_EVENT_TYPES,
  type TeamTemplate,
} from "@/components/team/teamTypes"

interface TemplateListViewProps {
  onEdit: (templateId: string | "__new__") => void
}

export default function TemplateListView({ onEdit }: TemplateListViewProps) {
  const { t } = useTranslation()
  const [templates, setTemplates] = useState<TeamTemplate[]>([])
  const [loading, setLoading] = useState(true)
  const [cloning, setCloning] = useState<string | null>(null)

  const reload = async () => {
    try {
      const list = (await getTransport().call("list_team_templates", {})) as TeamTemplate[]
      setTemplates(list)
    } catch (e) {
      logger.error("settings", "TemplateListView", "Failed to load templates", e)
    } finally {
      setLoading(false)
    }
  }
  const reloadEffectEvent = useEffectEvent(reload)

  useEffect(() => {
    reloadEffectEvent()
    const unsubscribe = getTransport().listen(TEAM_EVENT_CHANNEL, (payload) => {
      const event = payload as { type?: string } | undefined
      if (
        event?.type === TEAM_EVENT_TYPES.templateSaved ||
        event?.type === TEAM_EVENT_TYPES.templateDeleted
      ) {
        reloadEffectEvent()
      }
    })
    return unsubscribe
  }, [])

  const handleClone = async (tpl: TeamTemplate) => {
    const suffix = Date.now().toString(36).slice(-4)
    const cloned: TeamTemplate = {
      ...tpl,
      templateId: `${tpl.templateId}-copy-${suffix}`,
      name: `${tpl.name} (copy)`,
      createdAt: "",
      updatedAt: "",
    }
    setCloning(tpl.templateId)
    try {
      const saved = (await getTransport().call("save_team_template", {
        template: cloned,
      })) as TeamTemplate
      onEdit(saved.templateId)
    } catch (e) {
      logger.error("settings", "TemplateListView", "Clone failed", e)
    } finally {
      setCloning(null)
    }
  }

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
      <div className="w-full max-w-4xl mx-auto">
        <div className="flex items-center justify-between mb-5">
          <div>
            <h2 className="text-lg font-semibold text-foreground">{t("settings.teams")}</h2>
            <p className="text-xs text-muted-foreground mt-1">{t("settings.teamsDesc")}</p>
          </div>
          <Button size="sm" onClick={() => onEdit("__new__")}>
            <Plus className="h-3.5 w-3.5 mr-1" />
            {t("settings.teamNewTemplate")}
          </Button>
        </div>

        {templates.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border bg-secondary/10 p-8 text-center">
            <Users2 className="h-8 w-8 mx-auto text-muted-foreground/50 mb-2" />
            <p className="text-sm text-muted-foreground">{t("settings.teamsEmptyTitle")}</p>
            <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.teamsEmptyDesc")}</p>
          </div>
        ) : (
          <div className="space-y-2">
            {templates.map((tpl) => (
              <div
                key={tpl.templateId}
                className="group rounded-lg border border-border bg-secondary/20 hover:bg-secondary/40 transition-colors cursor-pointer"
                onClick={() => onEdit(tpl.templateId)}
              >
                <div className="p-3 flex items-center gap-3">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-sm truncate">{tpl.name}</span>
                      <span className="text-[10px] font-mono text-muted-foreground truncate">
                        {tpl.templateId}
                      </span>
                    </div>
                    {tpl.description && (
                      <p className="text-xs text-muted-foreground mt-0.5 line-clamp-1">
                        {tpl.description}
                      </p>
                    )}
                    <div className="flex items-center gap-1.5 mt-1.5">
                      {tpl.members.slice(0, 6).map((m, i) => (
                        <span
                          key={i}
                          className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-background border border-border"
                          style={{ borderLeftColor: m.color, borderLeftWidth: 2 }}
                        >
                          {m.name}
                        </span>
                      ))}
                      {tpl.members.length > 6 && (
                        <span className="text-[10px] text-muted-foreground">
                          +{tpl.members.length - 6}
                        </span>
                      )}
                    </div>
                  </div>
                  <IconTip label={t("settings.teamCloneTemplate")}>
                    <span className="inline-flex">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 opacity-0 group-hover:opacity-100 transition-opacity"
                        onClick={(e) => {
                          e.stopPropagation()
                          handleClone(tpl)
                        }}
                        disabled={cloning === tpl.templateId}
                      >
                        {cloning === tpl.templateId ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </span>
                  </IconTip>
                  <ChevronRight className="h-4 w-4 text-muted-foreground/50" />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
