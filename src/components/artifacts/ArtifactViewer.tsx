import { forwardRef, useEffect, useMemo, useState } from "react"
import { useTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { fileResourceAdapterFor } from "@/components/chat/files/fileResourceAdapter"
import type { FileTarget } from "@/components/chat/files/types"

interface ArtifactViewerProps {
  artifactId: string
  projectPath?: string | null
  title: string
  refreshKey?: string | number
  className?: string
}

/** Shared sandboxed reading surface for CanvasPanel and the Artifact Gallery. */
const ArtifactViewer = forwardRef<HTMLIFrameElement, ArtifactViewerProps>(
  ({ artifactId, projectPath, title, refreshKey = 0, className }, ref) => {
    const transport = useTransport()
    const target = useMemo<Extract<FileTarget, { kind: "artifact" }>>(
      () => ({
        kind: "artifact",
        artifactId,
        name: `${title || "Artifact"}.html`,
        projectPath,
      }),
      [artifactId, projectPath, title],
    )
    const [src, setSrc] = useState("")

    useEffect(() => {
      let cancelled = false
      const source = fileResourceAdapterFor(target).previewSource(target, { transport })
      void source
        .rawUrl()
        .then((url) => {
          if (!cancelled) setSrc(url ?? "")
        })
        .catch(() => {
          if (!cancelled) setSrc("")
        })
      return () => {
        cancelled = true
      }
    }, [target, transport, refreshKey])

    return (
      <iframe
        ref={ref}
        key={`${artifactId}-${refreshKey}`}
        src={src}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        className={cn(
          "block h-full min-h-0 w-full min-w-0 max-w-full border-0 bg-white dark:bg-surface-app",
          className,
        )}
        title={title}
      />
    )
  },
)

ArtifactViewer.displayName = "ArtifactViewer"

export default ArtifactViewer
