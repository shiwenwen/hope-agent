import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import {
  Hand,
  Monitor,
  HardDrive,
  Workflow,
  AppWindow,
  MapPin,
  BookUser,
  CalendarDays,
  ListChecks,
  Image,
  Camera,
  Mic,
  Globe,
  Bluetooth,
  FolderOpen,
  ShieldCheck,
  ShieldAlert,
  RefreshCw,
  ExternalLink,
} from "lucide-react"

// "granted" | "not_granted" | "unknown"
type PermState = string

interface AllPermissions {
  accessibility: PermState
  screen_recording: PermState
  automation: PermState
  app_management: PermState
  full_disk_access: PermState
  location: PermState
  contacts: PermState
  calendar: PermState
  reminders: PermState
  photos: PermState
  camera: PermState
  microphone: PermState
  local_network: PermState
  bluetooth: PermState
  files_and_folders: PermState
}

interface PermissionItem {
  id: keyof AllPermissions
  icon: React.ReactNode
  labelKey: string
  descKey: string
}

const PERMISSION_ITEMS: PermissionItem[] = [
  // ── Core control ──
  {
    id: "accessibility",
    icon: <Hand className="h-5 w-5" />,
    labelKey: "settings.permAccessibility",
    descKey: "settings.permAccessibilityDesc",
  },
  {
    id: "screen_recording",
    icon: <Monitor className="h-5 w-5" />,
    labelKey: "settings.permScreenRecording",
    descKey: "settings.permScreenRecordingDesc",
  },
  {
    id: "automation",
    icon: <Workflow className="h-5 w-5" />,
    labelKey: "settings.permAutomation",
    descKey: "settings.permAutomationDesc",
  },
  {
    id: "app_management",
    icon: <AppWindow className="h-5 w-5" />,
    labelKey: "settings.permAppManagement",
    descKey: "settings.permAppManagementDesc",
  },
  {
    id: "full_disk_access",
    icon: <HardDrive className="h-5 w-5" />,
    labelKey: "settings.permFullDiskAccess",
    descKey: "settings.permFullDiskAccessDesc",
  },
  {
    id: "files_and_folders",
    icon: <FolderOpen className="h-5 w-5" />,
    labelKey: "settings.permFilesAndFolders",
    descKey: "settings.permFilesAndFoldersDesc",
  },
  // ── Data access ──
  {
    id: "contacts",
    icon: <BookUser className="h-5 w-5" />,
    labelKey: "settings.permContacts",
    descKey: "settings.permContactsDesc",
  },
  {
    id: "calendar",
    icon: <CalendarDays className="h-5 w-5" />,
    labelKey: "settings.permCalendar",
    descKey: "settings.permCalendarDesc",
  },
  {
    id: "reminders",
    icon: <ListChecks className="h-5 w-5" />,
    labelKey: "settings.permReminders",
    descKey: "settings.permRemindersDesc",
  },
  {
    id: "photos",
    icon: <Image className="h-5 w-5" />,
    labelKey: "settings.permPhotos",
    descKey: "settings.permPhotosDesc",
  },
  // ── Devices & sensors ──
  {
    id: "camera",
    icon: <Camera className="h-5 w-5" />,
    labelKey: "settings.permCamera",
    descKey: "settings.permCameraDesc",
  },
  {
    id: "microphone",
    icon: <Mic className="h-5 w-5" />,
    labelKey: "settings.permMicrophone",
    descKey: "settings.permMicrophoneDesc",
  },
  {
    id: "location",
    icon: <MapPin className="h-5 w-5" />,
    labelKey: "settings.permLocation",
    descKey: "settings.permLocationDesc",
  },
  {
    id: "local_network",
    icon: <Globe className="h-5 w-5" />,
    labelKey: "settings.permLocalNetwork",
    descKey: "settings.permLocalNetworkDesc",
  },
  {
    id: "bluetooth",
    icon: <Bluetooth className="h-5 w-5" />,
    labelKey: "settings.permBluetooth",
    descKey: "settings.permBluetoothDesc",
  },
]

// ── Style helpers for three states ──

function stateBorder(state: PermState) {
  if (state === "granted") return "border-green-500/20 bg-green-500/5"
  if (state === "unknown") return "border-muted-foreground/20 bg-muted/30"
  return "border-amber-500/20 bg-amber-500/5"
}

function stateIconColor(state: PermState) {
  if (state === "granted") return "text-green-500"
  if (state === "unknown") return "text-muted-foreground"
  return "text-amber-500"
}

function stateBadgeClass(state: PermState) {
  if (state === "granted") return "bg-green-500/15 text-green-600 dark:text-green-400"
  if (state === "unknown") return "bg-muted text-muted-foreground"
  return "bg-amber-500/15 text-amber-600 dark:text-amber-400"
}

function stateBadgeKey(state: PermState) {
  if (state === "granted") return "settings.permGranted"
  if (state === "unknown") return "settings.permUnknown"
  return "settings.permNotGranted"
}

export default function PermissionsPanel() {
  const { t } = useTranslation()
  const [permissions, setPermissions] = useState<AllPermissions | null>(null)
  const [loading, setLoading] = useState(true)
  const [requesting, setRequesting] = useState<string | null>(null)

  const fetchPermissions = useCallback(async () => {
    try {
      setLoading(true)
      const result = await invoke<AllPermissions>("check_all_permissions")
      setPermissions(result)
    } catch (e) {
      logger.error("settings", "PermissionsPanel::fetch", "Failed to check permissions", e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchPermissions()
  }, [fetchPermissions])

  // Re-check when window regains focus
  useEffect(() => {
    const onFocus = () => fetchPermissions()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [fetchPermissions])

  const handleRequest = async (id: string) => {
    setRequesting(id)
    try {
      const result = await invoke<{ id: string; status: PermState }>("request_permission", { id })
      setPermissions((prev) => (prev ? { ...prev, [result.id]: result.status } : prev))
    } catch (e) {
      logger.error("settings", "PermissionsPanel::request", `Failed to request ${id}`, e)
    } finally {
      setRequesting(null)
    }
  }

  const grantedCount = permissions
    ? Object.values(permissions).filter((s) => s === "granted").length
    : 0
  const detectableCount = permissions
    ? Object.values(permissions).filter((s) => s !== "unknown").length
    : 0
  const allDetectableGranted = permissions
    ? Object.values(permissions).every((s) => s === "granted" || s === "unknown")
    : false

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      {/* Header summary */}
      <div className="flex items-center gap-3 mb-2">
        {allDetectableGranted && permissions ? (
          <ShieldCheck className="h-5 w-5 text-green-500" />
        ) : (
          <ShieldAlert className="h-5 w-5 text-amber-500" />
        )}
        <h3 className="text-sm font-semibold text-foreground">{t("settings.permTitle")}</h3>
      </div>
      <p className="text-xs text-muted-foreground mb-1">{t("settings.permDesc")}</p>
      {permissions && (
        <p className="text-xs text-muted-foreground mb-6">
          {t("settings.permSummary", { granted: grantedCount, total: detectableCount })}
        </p>
      )}

      {/* Permission list */}
      <div className="space-y-2">
        {PERMISSION_ITEMS.map((item) => {
          const state = permissions?.[item.id] ?? "not_granted"
          const isRequesting = requesting === item.id

          return (
            <div
              key={item.id}
              className={cn(
                "flex items-center gap-4 px-4 py-4 rounded-lg border transition-colors",
                stateBorder(state),
              )}
            >
              {/* Icon */}
              <span className={cn("shrink-0", stateIconColor(state))}>{item.icon}</span>

              {/* Label & description */}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-foreground">{t(item.labelKey)}</span>
                  <span
                    className={cn(
                      "text-[10px] font-medium px-1.5 py-0.5 rounded-full",
                      stateBadgeClass(state),
                    )}
                  >
                    {t(stateBadgeKey(state))}
                  </span>
                </div>
                <p className="text-xs text-muted-foreground mt-0.5">{t(item.descKey)}</p>
              </div>

              {/* Action button — show for not_granted and unknown */}
              {state !== "granted" && !loading && (
                <TooltipProvider>
                  <IconTip label={t("settings.permGrantTooltip")}>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={isRequesting}
                      onClick={() => handleRequest(item.id)}
                      className="shrink-0 gap-1.5"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                      {state === "unknown" ? t("settings.permCheck") : t("settings.permGrant")}
                    </Button>
                  </IconTip>
                </TooltipProvider>
              )}

              {state === "granted" && <ShieldCheck className="h-4 w-4 text-green-500 shrink-0" />}
            </div>
          )
        })}
      </div>

      {/* Refresh button */}
      <div className="mt-6 flex items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          disabled={loading}
          onClick={fetchPermissions}
          className="gap-1.5"
        >
          <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
          {t("settings.permRefresh")}
        </Button>
        <span className="text-xs text-muted-foreground">{t("settings.permRefreshHint")}</span>
      </div>
    </div>
  )
}
