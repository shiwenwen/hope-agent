import type { Message, Task, TaskStatus, ToolCall } from "@/types/chat"

export const TASK_TOOL_NAMES = new Set(["task_create", "task_update", "task_list"])

const TASK_STATUSES = new Set<TaskStatus>(["pending", "in_progress", "completed"])

export interface TaskProgressSnapshot {
  tasks: Task[]
  total: number
  completed: number
  remaining: number
  inProgress: boolean
}

type TranslationFn = (key: string, options?: Record<string, unknown>) => unknown

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}

export function isTaskToolName(name: string): boolean {
  return TASK_TOOL_NAMES.has(name)
}

function normalizeTask(value: unknown): Task | null {
  if (!isRecord(value)) return null

  const id = typeof value.id === "number" ? value.id : Number(value.id)
  if (!Number.isFinite(id)) return null

  const status = typeof value.status === "string" && TASK_STATUSES.has(value.status as TaskStatus)
    ? (value.status as TaskStatus)
    : "pending"

  return {
    id,
    sessionId: typeof value.sessionId === "string" ? value.sessionId : "",
    content: typeof value.content === "string" ? value.content : "",
    activeForm: typeof value.activeForm === "string" ? value.activeForm : null,
    batchId: typeof value.batchId === "string" ? value.batchId : null,
    status,
    createdAt: typeof value.createdAt === "string" ? value.createdAt : "",
    updatedAt: typeof value.updatedAt === "string" ? value.updatedAt : "",
  }
}

function normalizeTaskArray(value: unknown): Task[] | null {
  if (!Array.isArray(value)) return null
  return value.map(normalizeTask).filter((task): task is Task => task !== null)
}

function parseTaskToolResultInternal(result: string | undefined): Task[] | null {
  if (!result) return null
  try {
    const parsed = JSON.parse(result) as unknown
    if (Array.isArray(parsed)) return normalizeTaskArray(parsed)
    if (isRecord(parsed) && Array.isArray(parsed.tasks)) return normalizeTaskArray(parsed.tasks)
  } catch {
    return null
  }
  return null
}

export function parseTaskToolResult(result: string | undefined): Task[] {
  return parseTaskToolResultInternal(result) ?? []
}

export function createTaskProgressSnapshot(tasks: Task[]): TaskProgressSnapshot {
  const total = tasks.length
  const completed = tasks.filter((task) => task.status === "completed").length
  const remaining = Math.max(0, total - completed)
  const inProgress = tasks.some((task) => task.status === "in_progress")
  return { tasks, total, completed, remaining, inProgress }
}

const CURRENT_TASK_BATCH_WINDOW_MS = 2_000
const RECENT_COMPLETED_BATCH_MS = 30_000

function batchForAnchor(tasks: Task[], anchor: Task): Task[] {
  if (anchor.batchId) {
    const batch = tasks.filter((task) => task.batchId === anchor.batchId)
    if (batch.length > 0) return batch
  }

  const anchorCreatedAt = Date.parse(anchor.createdAt)
  if (!Number.isFinite(anchorCreatedAt)) return [anchor]

  const batch = tasks.filter((task) => {
    const createdAt = Date.parse(task.createdAt)
    return Number.isFinite(createdAt) && Math.abs(createdAt - anchorCreatedAt) <= CURRENT_TASK_BATCH_WINDOW_MS
  })
  return batch.length > 0 ? batch : [anchor]
}

export function selectCurrentTaskBatch(tasks: Task[]): Task[] {
  if (tasks.length <= 1) return tasks

  const activeTasks = tasks.filter((task) => task.status !== "completed")
  if (activeTasks.length > 0) {
    const anchor = activeTasks.reduce((latest, task) => task.id > latest.id ? task : latest)
    return batchForAnchor(tasks, anchor)
  }

  const anchor = tasks.reduce((latest, task) => task.id > latest.id ? task : latest)
  const updatedAt = Date.parse(anchor.updatedAt || anchor.createdAt)
  if (Number.isFinite(updatedAt) && Date.now() - updatedAt <= RECENT_COMPLETED_BATCH_MS) {
    return batchForAnchor(tasks, anchor)
  }

  return tasks
}

export function createCurrentTaskProgressSnapshot(tasks: Task[]): TaskProgressSnapshot {
  return createTaskProgressSnapshot(selectCurrentTaskBatch(tasks))
}

function* iterateTaskToolsReverse(message: Message): Generator<ToolCall> {
  if (message.contentBlocks?.length) {
    for (let i = message.contentBlocks.length - 1; i >= 0; i--) {
      const block = message.contentBlocks[i]
      if (block.type === "tool_call" && isTaskToolName(block.tool.name)) {
        yield block.tool
      }
    }
    return
  }
  const tools = message.toolCalls
  if (!tools) return
  for (let i = tools.length - 1; i >= 0; i--) {
    if (isTaskToolName(tools[i].name)) yield tools[i]
  }
}

export function extractLatestTaskProgressSnapshot(messages: Message[]): TaskProgressSnapshot | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    for (const tool of iterateTaskToolsReverse(messages[i])) {
      const parsed = parseTaskToolResultInternal(tool.result)
      if (parsed !== null) return createTaskProgressSnapshot(parsed)
    }
  }
  return null
}

export function taskProgressSnapshotFromTasks(value: unknown): TaskProgressSnapshot | null {
  const tasks = normalizeTaskArray(value)
  return tasks ? createTaskProgressSnapshot(tasks) : null
}

export function shouldShowTaskProgressPanel(
  snapshot: TaskProgressSnapshot | null | undefined,
  loading: boolean,
): snapshot is TaskProgressSnapshot {
  return !!snapshot && snapshot.total > 0 && (loading || snapshot.remaining > 0)
}

export function getTaskDisplayLabel(task: Task, fallback: string): string {
  const content = typeof task.content === "string" ? task.content.trim() : ""
  const activeForm = typeof task.activeForm === "string" ? task.activeForm.trim() : ""
  return task.status === "in_progress"
    ? activeForm || content || fallback
    : content || activeForm || fallback
}

export function getTaskProgressSummaryText(
  snapshot: TaskProgressSnapshot | null | undefined,
  t: TranslationFn,
): string {
  if (!snapshot || snapshot.total === 0) return String(t("executionStatus.task.empty"))
  if (snapshot.inProgress) {
    return String(t("executionStatus.task.running", {
      completed: snapshot.completed,
      total: snapshot.total,
      remaining: snapshot.remaining,
    }))
  }
  if (snapshot.completed === snapshot.total) {
    return String(t("executionStatus.task.completed", {
      completed: snapshot.completed,
      total: snapshot.total,
      remaining: snapshot.remaining,
    }))
  }
  return String(t("executionStatus.task.pending", {
    completed: snapshot.completed,
    total: snapshot.total,
    remaining: snapshot.remaining,
  }))
}
