import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Eye, EyeOff, Globe, Key, Plus, Trash2 } from "lucide-react"
import type { AuthProfile } from "./types"

function generateId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
}

export default function AuthProfileEditor({
  profiles,
  onChange,
}: {
  profiles: AuthProfile[]
  onChange: (profiles: AuthProfile[]) => void
}) {
  const { t } = useTranslation()
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set())

  function toggleKeyVisibility(id: string) {
    setVisibleKeys((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  function addProfile() {
    onChange([
      ...profiles,
      {
        id: generateId(),
        label: "",
        apiKey: "",
        enabled: true,
      },
    ])
  }

  function removeProfile(id: string) {
    onChange(profiles.filter((p) => p.id !== id))
  }

  function updateProfile(id: string, patch: Partial<AuthProfile>) {
    onChange(profiles.map((p) => (p.id === id ? { ...p, ...patch } : p)))
  }

  return (
    <div className="space-y-2.5">
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
          <Key className="h-3 w-3" />
          {t("authProfiles.title")}
        </label>
        <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={addProfile}>
          <Plus className="h-3 w-3 mr-1" />
          {t("authProfiles.add")}
        </Button>
      </div>

      {profiles.length === 0 && (
        <p className="text-[11px] text-muted-foreground/60 px-1">
          {t("authProfiles.empty")}
        </p>
      )}

      {profiles.map((profile) => (
        <div
          key={profile.id}
          className="bg-background border border-border rounded-lg p-3 space-y-2"
        >
          <div className="flex items-center justify-between gap-2">
            <Input
              value={profile.label}
              onChange={(e) => updateProfile(profile.id, { label: e.target.value })}
              placeholder={t("authProfiles.labelPlaceholder")}
              className="h-7 text-xs flex-1"
            />
            <div className="flex items-center gap-1.5">
              <Switch
                checked={profile.enabled}
                onCheckedChange={(checked) => updateProfile(profile.id, { enabled: checked })}
                className="scale-75"
              />
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 text-muted-foreground hover:text-red-400"
                onClick={() => removeProfile(profile.id)}
              >
                <Trash2 className="h-3 w-3" />
              </Button>
            </div>
          </div>

          <div className="relative">
            <Input
              type={visibleKeys.has(profile.id) ? "text" : "password"}
              value={profile.apiKey}
              onChange={(e) => updateProfile(profile.id, { apiKey: e.target.value })}
              placeholder="API Key"
              className="h-7 text-xs font-mono pr-8"
            />
            <button
              type="button"
              onClick={() => toggleKeyVisibility(profile.id)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
            >
              {visibleKeys.has(profile.id) ? (
                <EyeOff className="h-3 w-3" />
              ) : (
                <Eye className="h-3 w-3" />
              )}
            </button>
          </div>

          <div className="relative">
            <Input
              value={profile.baseUrl ?? ""}
              onChange={(e) =>
                updateProfile(profile.id, {
                  baseUrl: e.target.value || undefined,
                })
              }
              placeholder={t("authProfiles.baseUrlPlaceholder")}
              className="h-7 text-xs font-mono pl-7"
            />
            <Globe className="absolute left-2 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground/50" />
          </div>
        </div>
      ))}
    </div>
  )
}
