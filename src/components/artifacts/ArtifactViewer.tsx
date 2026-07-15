import { forwardRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

interface ArtifactViewerProps {
  projectPath?: string | null
  title: string
  refreshKey?: string | number
  className?: string
}

/** Shared sandboxed reading surface for CanvasPanel and the Artifact Gallery. */
const ArtifactViewer = forwardRef<HTMLIFrameElement, ArtifactViewerProps>(
  ({ projectPath, title, refreshKey = 0, className }, ref) => {
    const indexPath = projectPath ? `${projectPath}/index.html` : ""
    const src = indexPath ? (getTransport().resolveAssetUrl(indexPath) ?? "") : ""
    return (
      <iframe
        ref={ref}
        key={`${projectPath ?? "missing"}-${refreshKey}`}
        src={src}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        className={cn("h-full w-full border-0 bg-white dark:bg-surface-app", className)}
        title={title}
      />
    )
  },
)

ArtifactViewer.displayName = "ArtifactViewer"

export default ArtifactViewer
