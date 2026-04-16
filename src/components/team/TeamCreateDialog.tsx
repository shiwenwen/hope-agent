import { useState, useCallback, useEffect } from "react"
import { Plus, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { getTransport } from "@/lib/transport"
import type { TeamTemplate, MemberRole } from "./teamTypes"
import { TeamTemplateCard } from "./TeamTemplateCard"

interface TeamCreateDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated?: (teamId: string) => void
}

interface CustomMemberRow {
  name: string
  role: MemberRole
  task: string
}

const ROLE_OPTIONS: MemberRole[] = ["lead", "worker", "reviewer"]

export function TeamCreateDialog({
  open,
  onOpenChange,
  onCreated,
}: TeamCreateDialogProps) {
  const { t } = useTranslation()
  const [templates, setTemplates] = useState<TeamTemplate[]>([])
  const [mode, setMode] = useState<"template" | "custom">("template")
  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(null)

  useEffect(() => {
    if (open) {
      getTransport()
        .call<TeamTemplate[]>("list_team_templates", {})
        .then(setTemplates)
        .catch(() => {})
    }
  }, [open])
  const [teamName, setTeamName] = useState("")
  const [customMembers, setCustomMembers] = useState<CustomMemberRow[]>([
    { name: "", role: "lead", task: "" },
  ])
  const [creating, setCreating] = useState(false)

  const addMember = useCallback(() => {
    setCustomMembers((prev) => [
      ...prev,
      { name: "", role: "worker", task: "" },
    ])
  }, [])

  const removeMember = useCallback((idx: number) => {
    setCustomMembers((prev) => prev.filter((_, i) => i !== idx))
  }, [])

  const updateMember = useCallback(
    (idx: number, field: keyof CustomMemberRow, value: string) => {
      setCustomMembers((prev) =>
        prev.map((m, i) =>
          i === idx ? { ...m, [field]: value } : m,
        ),
      )
    },
    [],
  )

  const canCreate =
    mode === "template"
      ? selectedTemplate !== null
      : teamName.trim() !== "" && customMembers.some((m) => m.name.trim())

  const handleCreate = useCallback(async () => {
    if (!canCreate) return
    setCreating(true)
    try {
      const payload =
        mode === "template"
          ? { templateId: selectedTemplate }
          : {
              name: teamName.trim(),
              members: customMembers
                .filter((m) => m.name.trim())
                .map((m) => ({
                  name: m.name.trim(),
                  role: m.role,
                  task: m.task.trim() || undefined,
                })),
            }

      const result = await getTransport().call<{ teamId: string }>(
        "create_team",
        payload,
      )
      onCreated?.(result.teamId)
      onOpenChange(false)
    } catch {
      // Error handled by transport layer
    } finally {
      setCreating(false)
    }
  }, [canCreate, mode, selectedTemplate, teamName, customMembers, onCreated, onOpenChange])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("team.createTeam", "Create Team")}</DialogTitle>
          <DialogDescription>
            {t("team.createDesc", "Start a new agent team from a template or build one from scratch.")}
          </DialogDescription>
        </DialogHeader>

        <Tabs
          value={mode}
          onValueChange={(v) => setMode(v as "template" | "custom")}
        >
          <TabsList className="w-full">
            <TabsTrigger value="template" className="flex-1">
              {t("team.template", "Template")}
            </TabsTrigger>
            <TabsTrigger value="custom" className="flex-1">
              {t("team.custom", "Custom")}
            </TabsTrigger>
          </TabsList>

          {/* Template mode */}
          <TabsContent value="template">
            <div className="grid grid-cols-2 gap-2 mt-2">
              {templates.map((tpl) => (
                <TeamTemplateCard
                  key={tpl.templateId}
                  template={tpl}
                  selected={selectedTemplate === tpl.templateId}
                  onSelect={() => setSelectedTemplate(tpl.templateId)}
                />
              ))}
            </div>
          </TabsContent>

          {/* Custom mode */}
          <TabsContent value="custom">
            <div className="flex flex-col gap-3 mt-2">
              <Input
                value={teamName}
                onChange={(e) => setTeamName(e.target.value)}
                placeholder={t("team.teamName", "Team name")}
              />

              <div className="flex flex-col gap-2">
                {customMembers.map((m, idx) => (
                  <div key={idx} className="flex items-center gap-2">
                    <Input
                      value={m.name}
                      onChange={(e) => updateMember(idx, "name", e.target.value)}
                      placeholder={t("team.memberName", "Name")}
                      className="flex-1"
                    />
                    <Select
                      value={m.role}
                      onValueChange={(v) => updateMember(idx, "role", v)}
                    >
                      <SelectTrigger className="w-[110px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {ROLE_OPTIONS.map((r) => (
                          <SelectItem key={r} value={r}>
                            {t(`team.${r}`, r)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Input
                      value={m.task}
                      onChange={(e) => updateMember(idx, "task", e.target.value)}
                      placeholder={t("team.initialTask", "Task")}
                      className="flex-1"
                    />
                    {customMembers.length > 1 && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-8 w-8 p-0 shrink-0"
                        onClick={() => removeMember(idx)}
                      >
                        <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                      </Button>
                    )}
                  </div>
                ))}
              </div>

              <Button
                variant="outline"
                size="sm"
                onClick={addMember}
                className="self-start"
              >
                <Plus className="mr-1 h-3.5 w-3.5" />
                {t("team.addMember", "Add Member")}
              </Button>
            </div>
          </TabsContent>
        </Tabs>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={creating}
          >
            {t("common.cancel", "Cancel")}
          </Button>
          <Button onClick={handleCreate} disabled={!canCreate || creating}>
            {creating
              ? t("common.creating", "Creating...")
              : t("team.create", "Create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
