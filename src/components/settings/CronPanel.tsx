import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import {
  Plus,
  Search,
  Play,
  Pause,
  Trash2,
  Zap,
  Pencil,
  ChevronRight,
} from "lucide-react"
import CronJobForm from "@/components/cron/CronJobForm"
import CronJobDetail from "@/components/cron/CronJobDetail"
import type { CronJob } from "@/components/cron/CronJobForm"
import { statusColor, formatSchedule } from "@/components/cron/CronJobForm"

export default function CronPanel() {
  const { t } = useTranslation()
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [search, setSearch] = useState("")
  const [statusFilter, setStatusFilter] = useState<string>("all")
  const [showForm, setShowForm] = useState(false)
  const [editingJob, setEditingJob] = useState<CronJob | null>(null)
  const [detailJobId, setDetailJobId] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const fetchJobs = useCallback(async () => {
    try {
      const result = await invoke<CronJob[]>("cron_list_jobs")
      setJobs(result)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { fetchJobs() }, [fetchJobs])

  // Listen for cron:run_completed events
  useEffect(() => {
    const unlisten = listen("cron:run_completed", () => { fetchJobs() })
    return () => { unlisten.then((f) => f()) }
  }, [fetchJobs])

  const filteredJobs = jobs.filter((job) => {
    if (search && !job.name.toLowerCase().includes(search.toLowerCase())) return false
    if (statusFilter !== "all" && job.status !== statusFilter) return false
    return true
  })

  async function handleToggle(job: CronJob) {
    const enabled = job.status !== "active"
    await invoke("cron_toggle_job", { id: job.id, enabled })
    fetchJobs()
  }

  async function handleDelete(job: CronJob) {
    await invoke("cron_delete_job", { id: job.id })
    fetchJobs()
  }

  async function handleRunNow(job: CronJob) {
    await invoke("cron_run_now", { id: job.id })
    setTimeout(fetchJobs, 2000)
  }

  function handleFormClose() {
    setShowForm(false)
    setEditingJob(null)
    fetchJobs()
  }

  if (detailJobId) {
    return (
      <>
        <CronJobDetail
          jobId={detailJobId}
          onBack={() => setDetailJobId(null)}
          onEdit={(job) => { setEditingJob(job); setShowForm(true); setDetailJobId(null) }}
          onRefresh={fetchJobs}
        />
        {showForm && (
          <CronJobForm
            job={editingJob}
            onSave={handleFormClose}
            onCancel={() => { setShowForm(false); setEditingJob(null) }}
          />
        )}
      </>
    )
  }

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-5 py-3 border-b border-border">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
          <Input
            className="pl-8 h-8 text-xs"
            placeholder={t("cron.searchPlaceholder")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <select
          className="h-8 text-xs rounded-md border border-border bg-background px-2"
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
        >
          <option value="all">{t("cron.filterAll")}</option>
          <option value="active">{t("cron.active")}</option>
          <option value="paused">{t("cron.paused")}</option>
          <option value="disabled">{t("cron.disabled")}</option>
          <option value="completed">{t("cron.completed")}</option>
        </select>
        <Button variant="outline" size="sm" className="h-8 text-xs gap-1" onClick={() => { setEditingJob(null); setShowForm(true) }}>
          <Plus className="h-3.5 w-3.5" />
          {t("cron.newJob")}
        </Button>
      </div>

      {/* Job List */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
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
                <span className={`inline-block w-2 h-2 rounded-full shrink-0 ${statusColor(job.status)}`} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium truncate">{job.name}</div>
                  <div className="text-xs text-muted-foreground truncate">
                    {formatSchedule(job.schedule, t)}
                    {job.nextRunAt && ` · ${t("cron.nextRun")}: ${new Date(job.nextRunAt).toLocaleString()}`}
                  </div>
                </div>
                <div className="flex gap-0.5 shrink-0" onClick={(e) => e.stopPropagation()}>
                  <TooltipProvider delayDuration={100} skipDelayDuration={50}>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => handleRunNow(job)}>
                          <Zap className="h-3.5 w-3.5" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>{t("cron.runNow")}</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => { setEditingJob(job); setShowForm(true) }}>
                          <Pencil className="h-3.5 w-3.5" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>{t("common.edit")}</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => handleToggle(job)}>
                          {job.status === "active" ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>{job.status === "active" ? t("cron.pause") : t("cron.resume")}</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7 text-red-500 hover:text-red-600" onClick={() => handleDelete(job)}>
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>{t("common.delete")}</TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                </div>
                <ChevronRight className="h-4 w-4 text-muted-foreground shrink-0" />
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Form Modal */}
      {showForm && (
        <CronJobForm
          job={editingJob}
          onSave={handleFormClose}
          onCancel={() => { setShowForm(false); setEditingJob(null) }}
        />
      )}
    </div>
  )
}
