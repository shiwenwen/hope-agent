import React, { useEffect, useRef, useState } from "react"
import { X, ZoomIn, ZoomOut, RotateCcw } from "lucide-react"

interface ImageLightboxProps {
  src: string
  alt?: string
  onClose: () => void
}

function ImageLightbox({ src, alt, onClose }: ImageLightboxProps) {
  const [scale, setScale] = useState(1)
  const [translate, setTranslate] = useState({ x: 0, y: 0 })
  const [isDragging, setIsDragging] = useState(false)
  const dragging = useRef(false)
  const lastPos = useRef({ x: 0, y: 0 })
  const backdropRef = useRef<HTMLDivElement>(null)

  const handleWheel = (e: React.WheelEvent) => {
    e.stopPropagation()
    setScale((s) => Math.min(Math.max(s - e.deltaY * 0.001, 0.1), 10))
  }

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return
    dragging.current = true
    setIsDragging(true)
    lastPos.current = { x: e.clientX, y: e.clientY }
    ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
  }

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!dragging.current) return
    const dx = e.clientX - lastPos.current.x
    const dy = e.clientY - lastPos.current.y
    lastPos.current = { x: e.clientX, y: e.clientY }
    setTranslate((t) => ({ x: t.x + dx, y: t.y + dy }))
  }

  const handlePointerUp = () => {
    dragging.current = false
    setIsDragging(false)
  }

  const reset = () => {
    setScale(1)
    setTranslate({ x: 0, y: 0 })
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [onClose])

  return (
    <div
      ref={backdropRef}
      className="fixed inset-0 z-[9999] flex items-center justify-center bg-black/80 backdrop-blur-sm animate-in fade-in-0 duration-150"
      onClick={(e) => {
        if (e.target === backdropRef.current) onClose()
      }}
    >
      {/* Toolbar */}
      <div className="absolute top-4 right-4 flex items-center gap-1 z-10">
        <button
          onClick={() => setScale((s) => Math.min(s + 0.5, 10))}
          className="p-2 rounded-full bg-black/50 text-white/80 hover:text-white hover:bg-black/70 transition-colors"
        >
          <ZoomIn className="h-4 w-4" />
        </button>
        <button
          onClick={() => setScale((s) => Math.max(s - 0.5, 0.1))}
          className="p-2 rounded-full bg-black/50 text-white/80 hover:text-white hover:bg-black/70 transition-colors"
        >
          <ZoomOut className="h-4 w-4" />
        </button>
        <button
          onClick={reset}
          className="p-2 rounded-full bg-black/50 text-white/80 hover:text-white hover:bg-black/70 transition-colors"
        >
          <RotateCcw className="h-4 w-4" />
        </button>
        <button
          onClick={onClose}
          className="p-2 rounded-full bg-black/50 text-white/80 hover:text-white hover:bg-black/70 transition-colors"
        >
          <X className="h-5 w-5" />
        </button>
      </div>

      {/* Scale indicator */}
      {scale !== 1 && (
        <div className="absolute bottom-4 left-1/2 -translate-x-1/2 px-3 py-1 rounded-full bg-black/50 text-white/70 text-xs z-10">
          {Math.round(scale * 100)}%
        </div>
      )}

      {/* Image */}
      <img
        src={src}
        alt={alt || ""}
        draggable={false}
        className="max-w-[90vw] max-h-[90vh] object-contain select-none"
        style={{
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale})`,
          cursor: isDragging ? "grabbing" : "grab",
        }}
        onWheel={handleWheel}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onDoubleClick={() => {
          if (scale === 1) {
            setScale(2)
          } else {
            reset()
          }
        }}
      />
    </div>
  )
}

// ── Global lightbox context ────────────────────────────────────

interface LightboxState {
  src: string
  alt?: string
}

const LightboxContext = React.createContext<{
  openLightbox: (src: string, alt?: string) => void
}>({
  openLightbox: () => {},
})

// eslint-disable-next-line react-refresh/only-export-components
export function useLightbox() {
  return React.useContext(LightboxContext)
}

export function LightboxProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<LightboxState | null>(null)

  const openLightbox = (src: string, alt?: string) => {
    setState({ src, alt })
  }

  const closeLightbox = () => {
    setState(null)
  }

  // Global click delegation: intercept clicks on <img> inside markdown rendered areas
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement
      if (target.tagName !== "IMG") return
      if (!target.closest(".markdown-content")) return
      const img = target as HTMLImageElement
      if (img.src) {
        e.preventDefault()
        e.stopPropagation()
        setState({ src: img.src, alt: img.alt })
      }
    }
    document.addEventListener("click", handler, true)
    return () => document.removeEventListener("click", handler, true)
  }, [])

  return (
    <LightboxContext.Provider value={{ openLightbox }}>
      {children}
      {state && <ImageLightbox src={state.src} alt={state.alt} onClose={closeLightbox} />}
    </LightboxContext.Provider>
  )
}
