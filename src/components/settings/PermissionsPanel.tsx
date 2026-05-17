import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { isTauriMode } from "@/lib/transport"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  AppWindow,
  Bell,
  Bluetooth,
  BookUser,
  CalendarDays,
  Camera,
  CheckCircle2,
  Code2,
  ExternalLink,
  FolderOpen,
  Globe,
  Hand,
  HardDrive,
  HelpCircle,
  Home,
  Image,
  Info,
  Keyboard,
  ListChecks,
  MapPin,
  MessageSquare,
  Mic,
  Monitor,
  Music,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Volume2,
  Workflow,
  type LucideIcon,
} from "lucide-react"

type PermissionStatus =
  | "granted"
  | "not_granted"
  | "not_determined"
  | "restricted"
  | "manual_check"
  | "not_applicable"
  | "not_used"

type PermissionRequestMode = "native_prompt" | "open_settings" | "trigger_probe" | "none"

type PermissionGroup =
  | "control_capture"
  | "file_access"
  | "personal_data"
  | "device_network"
  | "system_services"

interface SystemPermissionItem {
  id: string
  group: PermissionGroup
  status: PermissionStatus
  requestMode: PermissionRequestMode
  settingsPane?: string | null
  usage: string
  note?: string | null
}

interface SystemPermissionsResponse {
  platform: string
  supported: boolean
  items: SystemPermissionItem[]
}

const GROUP_ORDER: PermissionGroup[] = [
  "control_capture",
  "file_access",
  "personal_data",
  "device_network",
  "system_services",
]

const GROUP_META: Record<PermissionGroup, { icon: LucideIcon; labelKey: string; fallback: string }> = {
  control_capture: {
    icon: Hand,
    labelKey: "settings.permissionGroups.controlCapture",
    fallback: "Control & Capture",
  },
  file_access: {
    icon: FolderOpen,
    labelKey: "settings.permissionGroups.fileAccess",
    fallback: "File Access",
  },
  personal_data: {
    icon: BookUser,
    labelKey: "settings.permissionGroups.personalData",
    fallback: "Personal Data",
  },
  device_network: {
    icon: Camera,
    labelKey: "settings.permissionGroups.deviceNetwork",
    fallback: "Devices & Network",
  },
  system_services: {
    icon: Bell,
    labelKey: "settings.permissionGroups.systemServices",
    fallback: "System Services",
  },
}

const ITEM_ICONS: Record<string, LucideIcon> = {
  accessibility: Hand,
  screen_recording: Monitor,
  system_audio_capture: Volume2,
  input_monitoring: Keyboard,
  automation_system_events: Workflow,
  automation_messages: MessageSquare,
  app_management: AppWindow,
  developer_tools: Code2,
  full_disk_access: HardDrive,
  desktop_folder: FolderOpen,
  documents_folder: FolderOpen,
  downloads_folder: FolderOpen,
  removable_volumes: HardDrive,
  network_volumes: Globe,
  location: MapPin,
  contacts: BookUser,
  calendar: CalendarDays,
  reminders: ListChecks,
  photos: Image,
  media_library: Music,
  speech_recognition: Mic,
  focus_status: Info,
  homekit: Home,
  camera: Camera,
  microphone: Mic,
  bluetooth: Bluetooth,
  local_network: Globe,
  notifications: Bell,
}

const STATUS_LABEL_KEYS: Record<PermissionStatus, string> = {
  granted: "settings.permissionStatuses.granted",
  not_granted: "settings.permissionStatuses.notGranted",
  not_determined: "settings.permissionStatuses.notDetermined",
  restricted: "settings.permissionStatuses.restricted",
  manual_check: "settings.permissionStatuses.manualCheck",
  not_applicable: "settings.permissionStatuses.notApplicable",
  not_used: "settings.permissionStatuses.notUsed",
}

const STATUS_FALLBACKS: Record<PermissionStatus, string> = {
  granted: "Granted",
  not_granted: "Not granted",
  not_determined: "Not determined",
  restricted: "Restricted",
  manual_check: "Manual check",
  not_applicable: "Not applicable",
  not_used: "Not used",
}

function stateBorder(state: PermissionStatus) {
  if (state === "granted") return "border-green-500/20 bg-green-500/5"
  if (state === "manual_check") return "border-sky-500/20 bg-sky-500/5"
  if (state === "not_applicable" || state === "not_used") return "border-muted-foreground/15 bg-muted/20"
  return "border-amber-500/20 bg-amber-500/5"
}

function stateIconColor(state: PermissionStatus) {
  if (state === "granted") return "text-green-500"
  if (state === "manual_check") return "text-sky-500"
  if (state === "not_applicable" || state === "not_used") return "text-muted-foreground"
  return "text-amber-500"
}

function stateBadgeClass(state: PermissionStatus) {
  if (state === "granted") return "bg-green-500/15 text-green-600 dark:text-green-400"
  if (state === "manual_check") return "bg-sky-500/15 text-sky-600 dark:text-sky-400"
  if (state === "not_applicable" || state === "not_used") return "bg-muted text-muted-foreground"
  return "bg-amber-500/15 text-amber-600 dark:text-amber-400"
}

function isActionable(state: PermissionStatus) {
  return state === "not_granted" || state === "not_determined" || state === "restricted"
}

function canRequest(item: SystemPermissionItem) {
  return (
    item.requestMode !== "none" &&
    item.status !== "granted" &&
    item.status !== "not_applicable" &&
    item.status !== "not_used"
  )
}

function itemTextKey(id: string, field: "label" | "usage" | "note") {
  return `settings.permissionItems.${id}.${field}`
}

function fallbackLabel(id: string) {
  return id
    .split("_")
    .filter(Boolean)
    .map((part) => part[0]?.toUpperCase() + part.slice(1))
    .join(" ")
}

function DisabledPermissionsPage() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl rounded-lg border border-muted-foreground/20 bg-muted/20 p-5">
        <div className="mb-2 flex items-center gap-3">
          <ShieldAlert className="h-5 w-5 text-muted-foreground" />
          <h3 className="text-sm font-semibold text-foreground">
            {t("settings.permUnsupportedTitle", "System permissions are macOS-only for now")}
          </h3>
        </div>
        <p className="text-xs leading-5 text-muted-foreground">
          {t(
            "settings.permUnsupportedDesc",
            "This page currently supports the macOS desktop app only. Windows, Linux, and HTTP/server mode will get a separate permissions implementation later.",
          )}
        </p>
      </div>
    </div>
  )
}

export default function PermissionsPanel() {
  const { t } = useTranslation()
  const [response, setResponse] = useState<SystemPermissionsResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [requesting, setRequesting] = useState<string | null>(null)

  const fetchPermissions = useCallback(async () => {
    if (!isTauriMode()) {
      setLoading(false)
      return
    }

    try {
      setLoading(true)
      const result = await getTransport().call<SystemPermissionsResponse>("check_system_permissions")
      setResponse(result)
    } catch (e) {
      logger.error("settings", "PermissionsPanel::fetch", "Failed to check permissions", e)
      setResponse({ platform: "unknown", supported: false, items: [] })
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchPermissions()
  }, [fetchPermissions])

  useEffect(() => {
    const onFocus = () => fetchPermissions()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [fetchPermissions])

  const handleRequest = async (id: string) => {
    setRequesting(id)
    try {
      const result = await getTransport().call<SystemPermissionItem>("request_system_permission", { id })
      setResponse((prev) =>
        prev
          ? {
              ...prev,
              items: prev.items.map((item) => (item.id === result.id ? result : item)),
            }
          : prev,
      )
    } catch (e) {
      logger.error("settings", "PermissionsPanel::request", `Failed to request ${id}`, e)
    } finally {
      setRequesting(null)
    }
  }

  if (!isTauriMode()) {
    return <DisabledPermissionsPage />
  }

  if (!loading && response && !response.supported) {
    return <DisabledPermissionsPage />
  }

  const items = response?.items ?? []
  const groups = GROUP_ORDER.map((group) => ({
    group,
    items: items.filter((item) => item.group === group),
  })).filter((entry) => entry.items.length > 0)

  const summary = {
    granted: items.filter((item) => item.status === "granted").length,
    needsAction: items.filter((item) => isActionable(item.status)).length,
    manual: items.filter((item) => item.status === "manual_check").length,
    inactive: items.filter((item) => item.status === "not_applicable" || item.status === "not_used").length,
  }
  const allClear = items.length > 0 && summary.needsAction === 0 && summary.manual === 0

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mb-2 flex items-center gap-3">
        {allClear ? (
          <ShieldCheck className="h-5 w-5 text-green-500" />
        ) : (
          <ShieldAlert className="h-5 w-5 text-amber-500" />
        )}
        <h3 className="text-sm font-semibold text-foreground">{t("settings.permTitle")}</h3>
      </div>
      <p className="mb-1 text-xs text-muted-foreground">
        {t(
          "settings.permDesc",
          "Hope Agent shows the macOS permissions it may need. Items without reliable public status APIs are marked for manual confirmation.",
        )}
      </p>
      <p className="mb-6 text-xs text-muted-foreground">
        {loading
          ? t("settings.permLoading", "Checking permissions...")
          : t("settings.permSummaryV2", {
              granted: summary.granted,
              needsAction: summary.needsAction,
              manual: summary.manual,
              inactive: summary.inactive,
            })}
      </p>

      <div className="space-y-6">
        {groups.map(({ group, items: groupItems }) => {
          const meta = GROUP_META[group]
          const GroupIcon = meta.icon

          return (
            <section key={group} className="space-y-2">
              <div className="flex items-center gap-2">
                <GroupIcon className="h-4 w-4 text-muted-foreground" />
                <h4 className="text-xs font-semibold uppercase text-muted-foreground">
                  {t(meta.labelKey, meta.fallback)}
                </h4>
              </div>

              <div className="space-y-2">
                {groupItems.map((item) => {
                  const Icon = ITEM_ICONS[item.id] ?? meta.icon
                  const isRequesting = requesting === item.id
                  const label = t(itemTextKey(item.id, "label"), fallbackLabel(item.id))
                  const usage = t(itemTextKey(item.id, "usage"), item.usage)
                  const note = item.note ? t(itemTextKey(item.id, "note"), item.note) : null

                  return (
                    <div
                      key={item.id}
                      className={cn(
                        "flex items-start gap-4 rounded-lg border px-4 py-4 transition-colors",
                        stateBorder(item.status),
                      )}
                    >
                      <span className={cn("mt-0.5 shrink-0", stateIconColor(item.status))}>
                        <Icon className="h-5 w-5" />
                      </span>

                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-medium text-foreground">{label}</span>
                          <span
                            className={cn(
                              "rounded-full px-1.5 py-0.5 text-[10px] font-medium",
                              stateBadgeClass(item.status),
                            )}
                          >
                            {t(STATUS_LABEL_KEYS[item.status], STATUS_FALLBACKS[item.status])}
                          </span>
                        </div>
                        <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{usage}</p>
                        {note && <p className="mt-1 text-[11px] leading-4 text-muted-foreground">{note}</p>}
                      </div>

                      {canRequest(item) && !loading && (
                        <IconTip label={t("settings.permGrantTooltip")}>
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={isRequesting}
                            onClick={() => handleRequest(item.id)}
                            className="shrink-0 gap-1.5"
                          >
                            {item.requestMode === "native_prompt" ? (
                              <ShieldCheck className="h-3.5 w-3.5" />
                            ) : (
                              <ExternalLink className="h-3.5 w-3.5" />
                            )}
                            {item.requestMode === "native_prompt"
                              ? t("settings.permGrant")
                              : t("settings.permCheck")}
                          </Button>
                        </IconTip>
                      )}

                      {item.status === "granted" && <CheckCircle2 className="h-4 w-4 shrink-0 text-green-500" />}
                      {(item.status === "not_applicable" || item.status === "not_used") && (
                        <HelpCircle className="h-4 w-4 shrink-0 text-muted-foreground" />
                      )}
                    </div>
                  )
                })}
              </div>
            </section>
          )
        })}
      </div>

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
