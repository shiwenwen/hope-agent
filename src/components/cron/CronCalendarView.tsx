import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import {
  ChevronLeft,
  ChevronRight,
  Plus,
  CalendarDays,
  List as ListIcon,
  Search,
  Play,
  Pause,
  Trash2,
  Zap,
  Pencil,
} from "lucide-react"
import { cn } from "@/lib/utils"
import CronJobForm from "./CronJobForm"
import CronJobDetail from "./CronJobDetail"
import type { CronJob, CalendarEvent } from "./CronJobForm.types"
import { statusColor, formatSchedule } from "./cronHelpers"

type ViewMode = "calendar" | "list"

interface CronCalendarViewProps {
  onBack: () => void
  onNavigateToSession?: (sessionId: string) => void
}

export default function CronCalendarView({ onNavigateToSession }: CronCalendarViewProps) {
  const { t } = useTranslation()
  const [mode, setMode] = useState<ViewMode>("calendar")
  const [currentDate, setCurrentDate] = useState(new Date())
  const [events, setEvents] = useState<CalendarEvent[]>([])
  const [selectedDate, setSelectedDate] = useState<Date | null>(null)
  const [showForm, setShowForm] = useState(false)
  const [editingJob, setEditingJob] = useState<CronJob | null>(null)
  const [detailJobId, setDetailJobId] = useState<string | null>(null)

  // List-view state
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [jobsLoaded, setJobsLoaded] = useState(false)
  const [listLoading, setListLoading] = useState(false)
  const [search, setSearch] = useState("")
  const [statusFilter, setStatusFilter] = useState<string>("all")

  const year = currentDate.getFullYear()
  const month = currentDate.getMonth()

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
      const result = await getTransport().call<CronJob[]>("cron_list_jobs")
      setJobs(result)
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
  const filteredJobs = jobs.filter((job) => {
    if (search && !job.name.toLowerCase().includes(search.toLowerCase())) return false
    if (statusFilter !== "all" && job.status !== statusFilter) return false
    return true
  })

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

  async function handleToggle(job: CronJob) {
    const enabled = job.status !== "active"
    await getTransport().call("cron_toggle_job", { id: job.id, enabled })
    refreshAll()
  }

  async function handleDelete(job: CronJob) {
    await getTransport().call("cron_delete_job", { id: job.id })
    refreshAll()
  }

  async function handleRunNow(job: CronJob) {
    await getTransport().call("cron_run_now", { id: job.id })
    setTimeout(refreshAll, 2000)
  }

  if (detailJobId) {
    return (
      <div className="flex flex-col flex-1 min-w-0 h-full bg-background">
        <CronJobDetail
          jobId={detailJobId}
          onBack={() => setDetailJobId(null)}
          onEdit={handleEditJob}
          onRefresh={refreshAll}
          onViewSession={onNavigateToSession}
        />
        {showForm && (
          <CronJobForm
            job={editingJob}
            defaultDate={mode === "calendar" ? selectedDate : null}
            onSave={handleFormClose}
            onCancel={() => {
              setShowForm(false)
              setEditingJob(null)
            }}
          />
        )}
      </div>
    )
  }

  return (
    <div className="flex flex-col flex-1 min-w-0 h-full bg-background">
      {/* Top Bar */}
      <div
        className="flex items-center gap-3 px-5 py-3 border-b border-border shrink-0"
        data-tauri-drag-region
      >
        <CalendarDays className="h-5 w-5 text-primary" />
        <h2 className="text-sm font-semibold">{t("cron.title")}</h2>

        {/* View mode switcher */}
        <div className="flex items-center rounded-md border border-border p-0.5 bg-secondary/30">
          <Button
            variant="ghost"
            size="sm"
            className={cn(
              "h-6 text-xs gap-1 px-2",
              mode === "calendar" ? "bg-background shadow-sm" : "text-muted-foreground",
            )}
            onClick={() => setMode("calendar")}
          >
            <CalendarDays className="h-3.5 w-3.5" />
            {t("cron.viewCalendar")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className={cn(
              "h-6 text-xs gap-1 px-2",
              mode === "list" ? "bg-background shadow-sm" : "text-muted-foreground",
            )}
            onClick={() => setMode("list")}
          >
            <ListIcon className="h-3.5 w-3.5" />
            {t("cron.viewList")}
          </Button>
        </div>

        <div className="flex-1" />

        {mode === "calendar" ? (
          <>
            <div className="flex items-center gap-1">
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={goPrevMonth}>
                <ChevronLeft className="h-4 w-4" />
              </Button>
              <Button variant="ghost" size="sm" className="text-xs px-2 h-7" onClick={goToday}>
                {t("cron.today")}
              </Button>
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={goNextMonth}>
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
            <span className="text-sm font-medium min-w-[120px] text-center">
              {currentDate.toLocaleString(undefined, { year: "numeric", month: "long" })}
            </span>
          </>
        ) : (
          <>
            <div className="relative w-56">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
              <Input
                className="pl-8 h-7 text-xs"
                placeholder={t("cron.searchPlaceholder")}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <select
              className="h-7 text-xs rounded-md border border-border bg-background px-2"
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
            >
              <option value="all">{t("cron.filterAll")}</option>
              <option value="active">{t("cron.active")}</option>
              <option value="paused">{t("cron.paused")}</option>
              <option value="disabled">{t("cron.disabled")}</option>
              <option value="completed">{t("cron.completed")}</option>
            </select>
          </>
        )}

        <Button variant="outline" size="sm" className="h-7 text-xs gap-1" onClick={handleNewJob}>
          <Plus className="h-3.5 w-3.5" />
          {t("cron.newJob")}
        </Button>
      </div>

      {/* Main Area */}
      {mode === "calendar" ? (
        <div className="flex flex-1 min-h-0 overflow-hidden">
          {/* Calendar Grid */}
          <div className="flex-1 flex flex-col min-w-0 p-4">
            {/* Week header */}
            <div className="grid grid-cols-7 shrink-0 mb-1">
              {weekDays.map((d, i) => (
                <div key={i} className="text-center text-xs font-medium text-muted-foreground py-1">
                  {d}
                </div>
              ))}
            </div>

            {/* Days grid — 6 rows stretch to fill remaining height */}
            <div
              className="grid grid-cols-7 flex-1 min-h-0 gap-px bg-border/30 rounded-lg overflow-hidden"
              style={{ gridTemplateRows: "repeat(6, minmax(0, 1fr))" }}
            >
              {cells.map((day, i) => (
                <button
                  key={i}
                  className={`
                    p-1.5 text-left bg-card transition-colors overflow-hidden
                    ${day ? "hover:bg-secondary/50 cursor-pointer" : "bg-secondary/10 cursor-default"}
                    ${day && selectedDate?.getDate() === day ? "ring-2 ring-primary ring-inset" : ""}
                  `}
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
                              const dotColor =
                                evt.runLog?.status === "success"
                                  ? "bg-emerald-500"
                                  : evt.runLog?.status === "error"
                                    ? "bg-red-500"
                                    : statusColor(evt.status)
                              return (
                                <span
                                  key={j}
                                  className={`inline-block w-1.5 h-1.5 rounded-full ${dotColor}`}
                                  title={evt.jobName}
                                />
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
            <div className="w-72 border-l border-border flex flex-col bg-card shrink-0">
              <div className="px-4 py-3 border-b border-border shrink-0">
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
              <div className="flex-1 min-h-0 overflow-y-auto px-3 py-2">
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
                      return (
                        <button
                          key={`${evt.jobId}-${i}`}
                          className="w-full text-left rounded-lg border border-border p-2.5 hover:bg-secondary/50 transition-colors"
                          onClick={() => setDetailJobId(evt.jobId)}
                        >
                          <div className="flex items-center gap-2">
                            <span
                              className={`inline-block w-2 h-2 rounded-full shrink-0 ${
                                evt.runLog?.status === "success"
                                  ? "bg-emerald-500"
                                  : evt.runLog?.status === "error"
                                    ? "bg-red-500"
                                    : statusColor(evt.status)
                              }`}
                            />
                            <span className="text-xs font-medium truncate">{evt.jobName}</span>
                            <span className="text-[10px] text-muted-foreground ml-auto shrink-0">
                              {time}
                            </span>
                          </div>
                          {runStatus && (
                            <div
                              className={`text-[10px] mt-1 ${runStatus === "success" ? "text-emerald-500" : "text-red-500"}`}
                            >
                              {runStatus === "success" ? "✓ " : "✕ "}
                              {runStatus === "success"
                                ? t("cron.runStatusSuccess")
                                : t("cron.runStatusError")}
                              {evt.runLog?.durationMs
                                ? ` (${(evt.runLog.durationMs / 1000).toFixed(1)}s)`
                                : ""}
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
        /* List View */
        <div className="flex-1 overflow-y-auto">
          {listLoading && !jobsLoaded ? (
            <div className="flex items-center justify-center h-32">
              <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
            </div>
          ) : filteredJobs.length === 0 ? (
            <div className="text-center py-12 text-muted-foreground text-sm">
              {jobs.length === 0 ? t("cron.noJobs") : t("cron.noResults")}
            </div>
          ) : (
            <div className="divide-y divide-border">
              {filteredJobs.map((job) => (
                <div
                  key={job.id}
                  className="flex items-center gap-3 px-5 py-3 hover:bg-secondary/30 transition-colors cursor-pointer"
                  onClick={() => setDetailJobId(job.id)}
                >
                  <span
                    className={`inline-block w-2 h-2 rounded-full shrink-0 ${statusColor(job.status)}`}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium truncate">{job.name}</div>
                    <div className="text-xs text-muted-foreground truncate">
                      {formatSchedule(job.schedule, t)}
                      {job.nextRunAt &&
                        ` · ${t("cron.nextRun")}: ${new Date(job.nextRunAt).toLocaleString()}`}
                    </div>
                  </div>
                  <div className="flex gap-0.5 shrink-0" onClick={(e) => e.stopPropagation()}>
                    <IconTip label={t("cron.runNow")}>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => handleRunNow(job)}
                      >
                        <Zap className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                    <IconTip label={t("common.edit")}>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => handleEditJob(job)}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                    <IconTip label={job.status === "active" ? t("cron.pause") : t("cron.resume")}>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => handleToggle(job)}
                      >
                        {job.status === "active" ? (
                          <Pause className="h-3.5 w-3.5" />
                        ) : (
                          <Play className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </IconTip>
                    <IconTip label={t("common.delete")}>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-red-500 hover:text-red-600"
                        onClick={() => handleDelete(job)}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                  </div>
                  <ChevronRight className="h-4 w-4 text-muted-foreground shrink-0" />
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Form Modal */}
      {showForm && (
        <CronJobForm
          job={editingJob}
          defaultDate={mode === "calendar" ? selectedDate : null}
          onSave={handleFormClose}
          onCancel={() => {
            setShowForm(false)
            setEditingJob(null)
          }}
        />
      )}
    </div>
  )
}
