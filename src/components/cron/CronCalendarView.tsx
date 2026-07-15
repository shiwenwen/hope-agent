import { useState, useEffect, useCallback, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  ChevronLeft,
  ChevronRight,
  Plus,
  CalendarDays,
  List as ListIcon,
  MessagesSquare,
  Loader2,
  Search,
  Send,
  AlertTriangle,
  Settings,
} from "lucide-react"
import { cn } from "@/lib/utils"
import CronJobForm from "./CronJobForm"
import CronJobDetail from "./CronJobDetail"
import CronConversationsPanel from "./CronConversationsPanel"
import CronLoopBadge from "./CronLoopBadge"
import type { CronJob, CalendarEvent } from "./CronJobForm.types"
import {
  statusColor,
  statusLabel,
  runLogDotColor,
  runStatusDisplay,
  formatSchedule,
  deliveryStatusColor,
  cronDisplayTitle,
  cronDisplayStatus,
} from "./cronHelpers"
import type { ProjectMeta } from "@/types/project"
import type { AgentSummaryForSidebar } from "@/types/chat"
import type { SettingsSection } from "@/components/settings/types"

type ViewMode = "calendar" | "list" | "conversations"

const VIEW_MODE_STORAGE_KEY = "cron_view_mode"

// List mode renders jobs client-side (search + status filter run on the full
// set), so paginate the *rendered* rows: show this many, "load more" adds more.
const JOBS_PAGE = 100

function readStoredViewMode(): ViewMode {
  try {
    const v = window.localStorage.getItem(VIEW_MODE_STORAGE_KEY)
    if (v === "list" || v === "conversations" || v === "calendar") return v
  } catch {
    // localStorage may be unavailable (private mode) — fall through.
  }
  return "calendar"
}

interface CronCalendarViewProps {
  onBack: () => void
  defaultProjectId?: string | null
  /** Open the main Settings page deep-linked to a section (e.g. "cron"). */
  onOpenSettings?: (section: SettingsSection) => void
}

export default function CronCalendarView({
  defaultProjectId,
  onOpenSettings,
}: CronCalendarViewProps) {
  const { t } = useTranslation()
  // Remember the last mode the user left the cron panel in across re-entries.
  const [mode, setMode] = useState<ViewMode>(readStoredViewMode)
  const [currentDate, setCurrentDate] = useState(new Date())
  const [events, setEvents] = useState<CalendarEvent[]>([])
  const [selectedDate, setSelectedDate] = useState<Date | null>(null)
  const [showForm, setShowForm] = useState(false)
  const [editingJob, setEditingJob] = useState<CronJob | null>(null)
  const [detailJobId, setDetailJobId] = useState<string | null>(null)

  // List-view state
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [jobsLoaded, setJobsLoaded] = useState(false)
  const [listLoading, setListLoading] = useState(false)
  const [search, setSearch] = useState("")
  const [statusFilter, setStatusFilter] = useState<string>("all")
  const [visibleJobsCount, setVisibleJobsCount] = useState(JOBS_PAGE)
  const [pendingDeleteJob, setPendingDeleteJob] = useState<CronJob | null>(null)
  const [deletingJobId, setDeletingJobId] = useState<string | null>(null)
  // List view is a master-detail: this is the job selected in the left column,
  // rendered as an embedded CronJobDetail on the right (separate from the
  // calendar's full-screen detailJobId).
  const [selectedListJobId, setSelectedListJobId] = useState<string | null>(null)
  // Agents power message-bubble identities inside CronJobDetail. Fetched once
  // here (job-independent) and passed down, so re-selecting list rows — which
  // remounts CronJobDetail via key — doesn't refetch the roster on every click.
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])

  const year = currentDate.getFullYear()
  const month = currentDate.getMonth()

  // Persist the selected view mode so re-entering the panel restores it.
  useEffect(() => {
    try {
      window.localStorage.setItem(VIEW_MODE_STORAGE_KEY, mode)
    } catch {
      // localStorage may be unavailable (private mode) — ignore.
    }
  }, [mode])

  // Reset list-mode pagination when the filter inputs or the job set change.
  useEffect(() => {
    setVisibleJobsCount(JOBS_PAGE)
  }, [search, statusFilter, jobs])

  const fetchEvents = useCallback(async () => {
    try {
      const start = new Date(year, month, 1)
      const end = new Date(year, month + 1, 1)
      const result = await getTransport().call<CalendarEvent[]>("cron_get_calendar_events", {
        start: start.toISOString(),
        end: end.toISOString(),
      })
      setEvents(result)
    } catch {
      // ignore
    }
  }, [year, month])

  const fetchJobs = useCallback(async () => {
    setListLoading(true)
    try {
      const [result, projectList] = await Promise.all([
        getTransport().call<CronJob[]>("cron_list_jobs"),
        getTransport().call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true }),
      ])
      setJobs(result)
      setProjects(Array.isArray(projectList) ? projectList : [])
      setJobsLoaded(true)
    } catch {
      // ignore
    } finally {
      setListLoading(false)
    }
  }, [])

  const refreshAll = useCallback(() => {
    fetchEvents()
    if (jobsLoaded) fetchJobs()
  }, [fetchEvents, fetchJobs, jobsLoaded])

  useEffect(() => {
    fetchEvents()
  }, [fetchEvents])

  // Lazily load jobs on first switch to list mode
  useEffect(() => {
    if (mode === "list" && !jobsLoaded) {
      fetchJobs()
    }
  }, [mode, jobsLoaded, fetchJobs])

  // Listen for cron:run_completed events
  useEffect(() => {
    return getTransport().listen("cron:run_completed", () => {
      fetchEvents()
      if (jobsLoaded) fetchJobs()
    })
  }, [fetchEvents, fetchJobs, jobsLoaded])

  // Load the agent roster once (job-independent); shared by both the embedded
  // and full-screen CronJobDetail so row switches don't refetch it.
  useEffect(() => {
    getTransport()
      .call<AgentSummaryForSidebar[]>("list_agents")
      .then((list) => setAgents(Array.isArray(list) ? list : []))
      .catch(() => {})
  }, [])

  function goToday() {
    setCurrentDate(new Date())
    setSelectedDate(null)
  }

  function goPrevMonth() {
    setCurrentDate(new Date(year, month - 1, 1))
    setSelectedDate(null)
  }

  function goNextMonth() {
    setCurrentDate(new Date(year, month + 1, 1))
    setSelectedDate(null)
  }

  // Calendar grid computation
  const firstDay = new Date(year, month, 1)
  const lastDay = new Date(year, month + 1, 0)
  const startOffset = (firstDay.getDay() + 6) % 7 // Monday = 0
  const daysInMonth = lastDay.getDate()

  // Build grid: 6 rows x 7 cols
  const cells: (number | null)[] = []
  for (let i = 0; i < startOffset; i++) cells.push(null)
  for (let d = 1; d <= daysInMonth; d++) cells.push(d)
  while (cells.length < 42) cells.push(null)

  // Group events by day
  const eventsByDay = new Map<number, CalendarEvent[]>()
  for (const evt of events) {
    const d = new Date(evt.scheduledAt)
    if (d.getMonth() === month && d.getFullYear() === year) {
      const day = d.getDate()
      if (!eventsByDay.has(day)) eventsByDay.set(day, [])
      eventsByDay.get(day)!.push(evt)
    }
  }

  // Selected day events
  const selectedDayEvents = selectedDate ? (eventsByDay.get(selectedDate.getDate()) ?? []) : []

  const today = new Date()
  const isToday = (day: number) =>
    day === today.getDate() && month === today.getMonth() && year === today.getFullYear()

  const weekDays = [
    t("cron.weekMon"),
    t("cron.weekTue"),
    t("cron.weekWed"),
    t("cron.weekThu"),
    t("cron.weekFri"),
    t("cron.weekSat"),
    t("cron.weekSun"),
  ]

  // Filtered jobs for list view
  const filteredJobs = useMemo(
    () =>
      jobs.filter((job) => {
        if (search && !job.name.toLowerCase().includes(search.toLowerCase())) return false
        if (statusFilter !== "all" && cronDisplayStatus(job) !== statusFilter) return false
        return true
      }),
    [jobs, search, statusFilter],
  )
  const visibleJobs = filteredJobs.slice(0, visibleJobsCount)

  // Master-detail selection. Default to the first job when nothing is selected
  // (fills the right pane), but drop the selection ONLY when the selected job is
  // gone from the full `jobs` set (deleted) — never merely because search/filter
  // hid it. Otherwise typing in the search box would yank the user away from the
  // job whose run history they're reading.
  useEffect(() => {
    if (mode !== "list") return
    if (selectedListJobId && !jobs.some((j) => j.id === selectedListJobId)) {
      setSelectedListJobId(null)
      return
    }
    if (!selectedListJobId && filteredJobs.length > 0) {
      setSelectedListJobId(filteredJobs[0].id)
    }
  }, [mode, jobs, filteredJobs, selectedListJobId])
  const projectMap = useMemo(() => new Map(projects.map((p) => [p.id, p])), [projects])
  const projectLabel = (projectId?: string | null) => {
    if (!projectId) return t("cron.noProject")
    const project = projectMap.get(projectId)
    if (!project) return t("cron.missingProject")
    return project.name
  }

  function handleDayClick(day: number) {
    setSelectedDate(new Date(year, month, day))
    setDetailJobId(null)
  }

  function handleNewJob() {
    setEditingJob(null)
    setShowForm(true)
  }

  function handleEditJob(job: CronJob) {
    setEditingJob(job)
    setShowForm(true)
    setDetailJobId(null)
  }

  function handleFormClose() {
    setShowForm(false)
    setEditingJob(null)
    refreshAll()
  }

  function handleDelete(job: CronJob) {
    setPendingDeleteJob(job)
  }

  async function confirmDeleteJob() {
    if (!pendingDeleteJob) return

    const job = pendingDeleteJob
    setDeletingJobId(job.id)

    try {
      await getTransport().call("cron_delete_job", { id: job.id })
      setPendingDeleteJob(null)
      if (detailJobId === job.id) {
        setDetailJobId(null)
      }
      // Clear the list master-detail selection synchronously too, so the right
      // pane doesn't keep rendering CronJobDetail for the just-deleted job (and
      // flash its "job not found" state) until refreshAll's round-trip lands.
      if (selectedListJobId === job.id) {
        setSelectedListJobId(null)
      }
      refreshAll()
      toast.success(t("cron.deleteSuccess", { name: job.name }))
    } catch {
      toast.error(t("cron.deleteFailed", { name: job.name }))
    } finally {
      setDeletingJobId(null)
    }
  }

  const deleteUi = (
    <>
      <AlertDialog
        open={!!pendingDeleteJob}
        onOpenChange={(open) => {
          if (!open && !deletingJobId) setPendingDeleteJob(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("cron.deleteConfirmTitle", { name: pendingDeleteJob?.name ?? "" })}
            </AlertDialogTitle>
            <AlertDialogDescription>{t("cron.deleteConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={!!deletingJobId}>{t("common.cancel")}</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => void confirmDeleteJob()}
              disabled={!pendingDeleteJob || !!deletingJobId}
            >
              {deletingJobId ? <Loader2 className="h-4 w-4 animate-spin" /> : t("common.delete")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )

  if (detailJobId) {
    return (
      <>
        <div className="flex flex-col flex-1 min-w-0 h-full bg-background">
          <CronJobDetail
            jobId={detailJobId}
            agents={agents}
            onBack={() => setDetailJobId(null)}
            onEdit={handleEditJob}
            onDelete={handleDelete}
            onRefresh={refreshAll}
          />
          {showForm && (
            <CronJobForm
              job={editingJob}
              defaultDate={mode === "calendar" ? selectedDate : null}
              defaultProjectId={defaultProjectId}
              onSave={handleFormClose}
              onCancel={() => {
                setShowForm(false)
                setEditingJob(null)
              }}
            />
          )}
        </div>
        {deleteUi}
      </>
    )
  }

  return (
    <div className="flex flex-col flex-1 min-w-0 h-full bg-background">
      {/* Top Bar */}
      <div className="flex shrink-0 items-center gap-3 px-5 pb-3 pt-4" data-tauri-drag-region>
        <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <CalendarDays className="h-4 w-4" />
        </span>
        <h2 className="text-[15px] font-semibold tracking-tight">{t("cron.title")}</h2>

        {/* View mode switcher */}
        <div className="flex items-center rounded-xl bg-muted/50 p-1">
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={mode === "calendar"}
            className={cn(
              "h-7 gap-1.5 rounded-lg px-2.5 text-xs",
              mode === "calendar"
                ? "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                : "text-muted-foreground",
            )}
            onClick={() => setMode("calendar")}
          >
            <CalendarDays className="h-3.5 w-3.5" />
            {t("cron.viewCalendar")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={mode === "list"}
            className={cn(
              "h-7 gap-1.5 rounded-lg px-2.5 text-xs",
              mode === "list"
                ? "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                : "text-muted-foreground",
            )}
            onClick={() => setMode("list")}
          >
            <ListIcon className="h-3.5 w-3.5" />
            {t("cron.viewList")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={mode === "conversations"}
            className={cn(
              "h-7 gap-1.5 rounded-lg px-2.5 text-xs",
              mode === "conversations"
                ? "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                : "text-muted-foreground",
            )}
            onClick={() => setMode("conversations")}
          >
            <MessagesSquare className="h-3.5 w-3.5" />
            {t("cron.viewConversations")}
          </Button>
        </div>

        <div className="flex-1" />

        {mode === "calendar" && (
          <>
            <div className="flex items-center gap-0.5 rounded-xl bg-muted/35 p-0.5">
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 rounded-lg"
                onClick={goPrevMonth}
              >
                <ChevronLeft className="h-4 w-4" />
              </Button>
              <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={goToday}>
                {t("cron.today")}
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 rounded-lg"
                onClick={goNextMonth}
              >
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
            <span className="min-w-[120px] text-center text-sm font-medium">
              {currentDate.toLocaleString(undefined, { year: "numeric", month: "long" })}
            </span>
          </>
        )}

        {onOpenSettings && (
          <IconTip label={t("cron.openSettings")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => onOpenSettings("cron")}
            >
              <Settings className="h-4 w-4" />
            </Button>
          </IconTip>
        )}

        <Button size="sm" className="h-8 gap-1.5 rounded-lg px-3 text-xs" onClick={handleNewJob}>
          <Plus className="h-3.5 w-3.5" />
          {t("cron.newJob")}
        </Button>
      </div>

      {/* Main Area */}
      {mode === "conversations" ? (
        <CronConversationsPanel />
      ) : mode === "calendar" ? (
        <div className="flex flex-1 min-h-0 overflow-hidden">
          {/* Calendar Grid */}
          <div className="flex min-w-0 flex-1 flex-col px-5 pb-5 pt-2">
            {/* Week header */}
            <div className="grid grid-cols-7 shrink-0 mb-1">
              {weekDays.map((d, i) => (
                <div key={i} className="text-center text-xs font-medium text-muted-foreground py-1">
                  {d}
                </div>
              ))}
            </div>

            {/* Days grid — 6 rows stretch to fill remaining height */}
            <div className="grid min-h-0 flex-1 grid-cols-7 grid-rows-6 gap-1.5">
              {cells.map((day, i) => (
                <button
                  key={i}
                  className={cn(
                    "overflow-hidden rounded-xl p-2 text-left transition-colors",
                    day
                      ? "cursor-pointer bg-muted/20 hover:bg-muted/45"
                      : "cursor-default bg-transparent",
                    day && selectedDate?.getDate() === day && "bg-primary/10 hover:bg-primary/15",
                  )}
                  onClick={() => day && handleDayClick(day)}
                  disabled={!day}
                >
                  {day && (
                    <>
                      <span
                        className={`
                        text-xs font-medium inline-flex items-center justify-center
                        ${isToday(day) ? "bg-primary text-primary-foreground rounded-full w-5 h-5" : "text-foreground"}
                      `}
                      >
                        {day}
                      </span>
                      {/* Event dots */}
                      {eventsByDay.has(day) && (
                        <div className="flex gap-0.5 mt-1 flex-wrap">
                          {eventsByDay
                            .get(day)!
                            .slice(0, 4)
                            .map((evt, j) => {
                              const dotColor = runLogDotColor(evt.runLog?.status, evt.status)
                              const eventTitle = cronDisplayTitle(evt.jobName, evt.payloadType)
                              return (
                                <IconTip key={j} label={eventTitle}>
                                  <span
                                    className={`inline-block w-1.5 h-1.5 rounded-full ${dotColor}`}
                                  />
                                </IconTip>
                              )
                            })}
                          {eventsByDay.get(day)!.length > 4 && (
                            <span className="text-[9px] text-muted-foreground">
                              +{eventsByDay.get(day)!.length - 4}
                            </span>
                          )}
                        </div>
                      )}
                    </>
                  )}
                </button>
              ))}
            </div>
          </div>

          {/* Day Detail Sidebar */}
          {selectedDate && (
            <div className="m-3 ml-0 flex w-72 shrink-0 flex-col overflow-hidden rounded-2xl bg-muted/25">
              <div className="shrink-0 px-4 pb-2 pt-4">
                <h3 className="text-sm font-medium">
                  {selectedDate.toLocaleDateString(undefined, {
                    weekday: "long",
                    month: "long",
                    day: "numeric",
                  })}
                </h3>
                <p className="text-xs text-muted-foreground mt-0.5">
                  {selectedDayEvents.length} {t("cron.tasks")}
                </p>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-2.5">
                {selectedDayEvents.length === 0 ? (
                  <p className="text-xs text-muted-foreground py-6 text-center">
                    {t("cron.noTasksThisDay")}
                  </p>
                ) : (
                  <div className="space-y-1.5">
                    {selectedDayEvents.map((evt, i) => {
                      const time = new Date(evt.scheduledAt).toLocaleTimeString(undefined, {
                        hour: "2-digit",
                        minute: "2-digit",
                      })
                      const runStatus = evt.runLog?.status
                      const runDisp = runStatus ? runStatusDisplay(runStatus) : null
                      const eventTitle = cronDisplayTitle(evt.jobName, evt.payloadType)
                      const isLoop = evt.payloadType === "sessionLoop"
                      return (
                        <button
                          key={`${evt.jobId}-${i}`}
                          className="w-full rounded-xl bg-background/70 p-3 text-left transition-colors hover:bg-background"
                          onClick={() => setDetailJobId(evt.jobId)}
                        >
                          <div className="flex items-center gap-2">
                            <span
                              className={`inline-block w-2 h-2 rounded-full shrink-0 ${runLogDotColor(
                                evt.runLog?.status,
                                evt.status,
                              )}`}
                            />
                            {isLoop && <CronLoopBadge />}
                            <span className="truncate text-xs font-medium">{eventTitle}</span>
                            <span className="text-[10px] text-muted-foreground ml-auto shrink-0">
                              {time}
                            </span>
                          </div>
                          {runDisp && (
                            <div className={`text-[10px] mt-1 ${runDisp.className}`}>
                              {runDisp.symbol}
                              {t(runDisp.labelKey)}
                              {evt.runLog?.durationMs
                                ? ` (${(evt.runLog.durationMs / 1000).toFixed(1)}s)`
                                : ""}
                            </div>
                          )}
                          {evt.runLog?.deliveryStatus && (
                            <div className="mt-0.5 flex items-center gap-1 text-[10px]">
                              <Send
                                className={cn(
                                  "h-2.5 w-2.5",
                                  deliveryStatusColor(evt.runLog.deliveryStatus),
                                )}
                              />
                              <span className={deliveryStatusColor(evt.runLog.deliveryStatus)}>
                                {t(`cron.deliveryStatus.${evt.runLog.deliveryStatus}`)}
                              </span>
                            </div>
                          )}
                        </button>
                      )
                    })}
                  </div>
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-full mt-2 text-xs gap-1"
                  onClick={handleNewJob}
                >
                  <Plus className="h-3.5 w-3.5" />
                  {t("cron.newJob")}
                </Button>
              </div>
            </div>
          )}
        </div>
      ) : (
        /* List View — master-detail: left job list · right embedded detail */
        <div className="flex min-h-0 flex-1 px-3 pb-3">
          {/* Left — job list */}
          <div className="flex w-[19.5rem] shrink-0 flex-col pr-3">
            <div className="flex shrink-0 items-center gap-2 px-1 pb-2 pt-1">
              <div className="relative min-w-0 flex-1">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <SearchInput
                  className="h-8 rounded-lg pl-8 text-xs"
                  placeholder={t("cron.searchPlaceholder")}
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                />
              </div>
              <Select value={statusFilter} onValueChange={setStatusFilter}>
                <SelectTrigger className="h-8 w-auto min-w-[6.5rem] max-w-[10rem] shrink-0 px-2 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all" className="text-xs">
                    {t("cron.filterAll")}
                  </SelectItem>
                  <SelectItem value="active" className="text-xs">
                    {t("cron.active")}
                  </SelectItem>
                  <SelectItem value="paused" className="text-xs">
                    {t("cron.paused")}
                  </SelectItem>
                  <SelectItem value="disabled" className="text-xs">
                    {t("cron.disabled")}
                  </SelectItem>
                  <SelectItem value="completed" className="text-xs">
                    {t("cron.completed")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="flex-1 overflow-y-auto">
              {listLoading && !jobsLoaded ? (
                <div className="flex items-center justify-center py-10 text-muted-foreground">
                  <Loader2 className="h-5 w-5 animate-spin" />
                </div>
              ) : filteredJobs.length === 0 ? (
                <div className="px-4 py-10 text-center text-xs text-muted-foreground">
                  {jobs.length === 0 ? t("cron.noJobs") : t("cron.noResults")}
                </div>
              ) : (
                <div className="space-y-1">
                  {visibleJobs.map((job) => {
                    const isActive = job.id === selectedListJobId
                    const isLoop = job.payload.type === "sessionLoop"
                    const title = cronDisplayTitle(job.name, job.payload.type)
                    const displayStatus = cronDisplayStatus(job)
                    return (
                      <button
                        key={job.id}
                        onClick={() => setSelectedListJobId(job.id)}
                        className={cn(
                          "w-full rounded-xl px-3 py-3 text-left transition-colors",
                          isActive ? "bg-sky-500/[0.075]" : "hover:bg-sky-500/[0.04]",
                        )}
                      >
                        <div className="flex items-center gap-2">
                          <IconTip label={statusLabel(displayStatus, t)}>
                            <span
                              className={cn(
                                "inline-block h-2 w-2 shrink-0 rounded-full",
                                statusColor(displayStatus),
                              )}
                            />
                          </IconTip>
                          <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs font-medium">
                            {isLoop && <CronLoopBadge />}
                            <span className="truncate">{title}</span>
                          </span>
                        </div>
                        <div className="mt-1.5 truncate pl-4 text-[10px] text-muted-foreground">
                          {formatSchedule(job.schedule, t)}
                          {` · ${projectLabel(job.projectId)}`}
                        </div>
                        {job.nextRunAt && (
                          <div className="mt-0.5 truncate pl-4 text-[10px] text-muted-foreground">
                            {t("cron.nextRun")}: {new Date(job.nextRunAt).toLocaleString()}
                          </div>
                        )}
                        {(job.deliveryTargets.length > 0 || job.consecutiveFailures > 0) && (
                          <div className="mt-1 flex items-center gap-2 pl-4 text-[10px] text-muted-foreground">
                            {job.deliveryTargets.length > 0 && (
                              <span
                                className={cn(
                                  "inline-flex items-center gap-1",
                                  job.deliveryTargets.some((tg) => tg.stale) && "text-red-500",
                                )}
                              >
                                <Send className="h-3 w-3" />
                                {job.deliveryTargets.length}
                              </span>
                            )}
                            {job.consecutiveFailures > 0 && (
                              <span className="inline-flex items-center gap-1 text-amber-500">
                                <AlertTriangle className="h-3 w-3" />
                                {job.consecutiveFailures}/{job.maxFailures}
                              </span>
                            )}
                          </div>
                        )}
                      </button>
                    )
                  })}
                  {filteredJobs.length > visibleJobs.length && (
                    <div className="px-3 py-2">
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-7 w-full text-xs"
                        onClick={() => setVisibleJobsCount((n) => n + JOBS_PAGE)}
                      >
                        {t("cron.loadMore")}
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>

          {/* Right — embedded detail of the selected job */}
          <div className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background">
            {selectedListJobId ? (
              <CronJobDetail
                key={selectedListJobId}
                jobId={selectedListJobId}
                agents={agents}
                embedded
                onBack={() => setSelectedListJobId(null)}
                onEdit={handleEditJob}
                onDelete={handleDelete}
                onRefresh={refreshAll}
              />
            ) : listLoading && !jobsLoaded ? (
              <div className="flex flex-1 items-center justify-center text-muted-foreground">
                <Loader2 className="h-5 w-5 animate-spin" />
              </div>
            ) : (
              <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-muted-foreground">
                <ListIcon className="h-10 w-10 opacity-40" />
                <p className="text-sm">
                  {jobs.length === 0 ? t("cron.noJobs") : t("cron.noResults")}
                </p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Form Modal */}
      {showForm && (
        <CronJobForm
          job={editingJob}
          defaultDate={mode === "calendar" ? selectedDate : null}
          defaultProjectId={defaultProjectId}
          onSave={handleFormClose}
          onCancel={() => {
            setShowForm(false)
            setEditingJob(null)
          }}
        />
      )}
      {deleteUi}
    </div>
  )
}
