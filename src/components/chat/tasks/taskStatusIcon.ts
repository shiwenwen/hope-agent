import { CheckCircle, Circle, Loader2 } from "lucide-react"
import type { TaskStatus } from "@/types/chat"

export const TASK_STATUS_ICON: Record<TaskStatus, { Icon: typeof Circle; cls: string }> = {
  pending: { Icon: Circle, cls: "text-muted-foreground" },
  in_progress: { Icon: Loader2, cls: "animate-spin text-blue-500" },
  completed: { Icon: CheckCircle, cls: "text-green-500" },
}
