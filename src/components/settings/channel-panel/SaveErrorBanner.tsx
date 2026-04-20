import { AlertCircle } from "lucide-react"

export default function SaveErrorBanner({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <div className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      <AlertCircle className="h-4 w-4 mt-0.5 flex-shrink-0" />
      <span className="break-words">{message}</span>
    </div>
  )
}
