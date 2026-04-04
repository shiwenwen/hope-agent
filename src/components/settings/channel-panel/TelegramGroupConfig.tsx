import { useState } from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IconTip } from "@/components/ui/tooltip"
import { Plus, Trash2 } from "lucide-react"
import AgentAvatar from "./AgentAvatar"
import GroupConfigItem from "./GroupConfigItem"
import type { AgentInfo, TelegramGroupConfig as TelegramGroupConfigType, TelegramChannelConfig } from "./types"

export default function TelegramGroupChannelConfig({
  groupPolicy,
  onGroupPolicyChange,
  groups,
  onGroupsChange,
  channels,
  onChannelsChange,
  agents,
  t,
}: {
  groupPolicy: string
  onGroupPolicyChange: (v: string) => void
  groups: Record<string, TelegramGroupConfigType>
  onGroupsChange: (v: Record<string, TelegramGroupConfigType>) => void
  channels: Record<string, TelegramChannelConfig>
  onChannelsChange: (v: Record<string, TelegramChannelConfig>) => void
  agents: AgentInfo[]
  t: (key: string) => string
}) {
  const [newGroupId, setNewGroupId] = useState("")
  const [newChannelId, setNewChannelId] = useState("")

  const addGroup = () => {
    const id = newGroupId.trim()
    if (!id || id in groups) return
    onGroupsChange({
      ...groups,
      [id]: {
        requireMention: null,
        enabled: true,
        allowFrom: [],
        agentId: null,
        systemPrompt: null,
        topics: {},
      },
    })
    setNewGroupId("")
  }

  const removeGroup = (id: string) => {
    const next = { ...groups }
    delete next[id]
    onGroupsChange(next)
  }

  const updateGroup = (id: string, patch: Partial<TelegramGroupConfigType>) => {
    onGroupsChange({
      ...groups,
      [id]: { ...groups[id], ...patch },
    })
  }

  const addChannel = () => {
    const id = newChannelId.trim()
    if (!id || id in channels) return
    onChannelsChange({
      ...channels,
      [id]: { requireMention: null, enabled: true, agentId: null, systemPrompt: null },
    })
    setNewChannelId("")
  }

  const removeChannel = (id: string) => {
    const next = { ...channels }
    delete next[id]
    onChannelsChange(next)
  }

  const updateChannel = (id: string, patch: Partial<TelegramChannelConfig>) => {
    onChannelsChange({
      ...channels,
      [id]: { ...channels[id], ...patch },
    })
  }

  return (
    <>
      {/* Divider line */}
      <div className="border-t my-2" />

      {/* Group Policy */}
      <div className="space-y-2">
        <Label>{t("channels.groupPolicy")}</Label>
        <Select value={groupPolicy} onValueChange={onGroupPolicyChange}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="open">{t("channels.groupPolicyOpen")}</SelectItem>
            <SelectItem value="allowlist">{t("channels.groupPolicyAllowlist")}</SelectItem>
            <SelectItem value="disabled">{t("channels.groupPolicyDisabled")}</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Group Configuration List */}
      {groupPolicy !== "disabled" && (
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <Label>{t("channels.groupConfig")}</Label>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("channels.groupConfigHint")}
              </p>
            </div>
          </div>

          {/* Existing groups */}
          <div className="space-y-2">
            {Object.entries(groups).map(([gId, gCfg]) => (
              <GroupConfigItem
                key={gId}
                groupId={gId}
                config={gCfg}
                agents={agents}
                onUpdate={(patch) => updateGroup(gId, patch)}
                onRemove={() => removeGroup(gId)}
                t={t}
              />
            ))}
          </div>

          {/* Add group */}
          <div className="flex gap-2">
            <Input
              placeholder={t("channels.groupIdPlaceholder")}
              value={newGroupId}
              onChange={(e) => setNewGroupId(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addGroup() }}
              className="flex-1"
            />
            <Button variant="outline" size="sm" onClick={addGroup} disabled={!newGroupId.trim()}>
              <Plus className="h-4 w-4 mr-1" />
              {t("channels.addGroup")}
            </Button>
          </div>
        </div>
      )}

      {/* Divider */}
      <div className="border-t my-2" />

      {/* Channel Configuration List */}
      <div className="space-y-3">
        <div>
          <Label>{t("channels.channelConfig")}</Label>
          <p className="text-xs text-muted-foreground mt-0.5">
            {t("channels.channelConfigHint")}
          </p>
        </div>

        {/* Existing channels */}
        <div className="space-y-2">
          {Object.entries(channels).map(([cId, cCfg]) => (
            <div key={cId} className="rounded-lg border bg-card p-3 space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium font-mono">{cId}</span>
                <IconTip label={t("channels.removeConfig")}>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted"
                    onClick={() => removeChannel(cId)}
                  >
                    <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                  </button>
                </IconTip>
              </div>
              <div className="flex items-center gap-4 flex-wrap">
                <div className="flex items-center gap-2">
                  <Label className="text-xs">{t("channels.channelEnabled")}</Label>
                  <Switch
                    checked={cCfg.enabled !== false}
                    onCheckedChange={(v) => updateChannel(cId, { enabled: v })}
                  />
                </div>
                <div className="flex items-center gap-2">
                  <Label className="text-xs">{t("channels.groupRequireMention")}</Label>
                  <Select
                    value={cCfg.requireMention === null || cCfg.requireMention === undefined ? "yes" : cCfg.requireMention ? "yes" : "no"}
                    onValueChange={(v) => updateChannel(cId, { requireMention: v === "yes" })}
                  >
                    <SelectTrigger className="h-7 text-xs w-20">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="yes">✓</SelectItem>
                      <SelectItem value="no">✗</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="flex-1 min-w-[160px]">
                  <Select
                    value={cCfg.agentId || "__none__"}
                    onValueChange={(v) => updateChannel(cId, { agentId: v === "__none__" ? null : v })}
                  >
                    <SelectTrigger className="h-8 text-xs">
                      <SelectValue placeholder={t("channels.boundAgentDefault")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
                      {agents.map((a) => (
                        <SelectItem key={a.id} value={a.id}>
                          <span className="flex items-center gap-2">
                            <AgentAvatar agent={a} />
                            {a.name}
                          </span>
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* Add channel */}
        <div className="flex gap-2">
          <Input
            placeholder={t("channels.channelIdPlaceholder")}
            value={newChannelId}
            onChange={(e) => setNewChannelId(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") addChannel() }}
            className="flex-1"
          />
          <Button variant="outline" size="sm" onClick={addChannel} disabled={!newChannelId.trim()}>
            <Plus className="h-4 w-4 mr-1" />
            {t("channels.addChannel")}
          </Button>
        </div>
      </div>
    </>
  )
}
