import { useEffect, useState } from "react"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import { cn } from "@/lib/utils"

function readTheme(): "light" | "dark" {
  if (typeof document === "undefined") return "light"
  return document.documentElement.classList.contains("dark") ? "dark" : "light"
}

export function Toaster({
  closeButton = true,
  position = "bottom-right",
  toastOptions,
  ...props
}: ToasterProps) {
  const [theme, setTheme] = useState<"light" | "dark">(readTheme)

  useEffect(() => {
    const root = document.documentElement
    const observer = new MutationObserver(() => setTheme(readTheme()))
    observer.observe(root, {
      attributes: true,
      attributeFilter: ["class"],
    })

    return () => observer.disconnect()
  }, [])

  return (
    <Sonner
      {...props}
      closeButton={closeButton}
      position={position}
      theme={theme}
      toastOptions={{
        ...toastOptions,
        classNames: {
          toast: cn(
            "group rounded-xl border border-border bg-popover text-popover-foreground shadow-lg",
            "data-[type=success]:border-emerald-500/25 data-[type=success]:bg-emerald-500/10",
            "data-[type=error]:border-destructive/30 data-[type=error]:bg-destructive/10",
            toastOptions?.classNames?.toast,
          ),
          title: cn("text-sm font-medium", toastOptions?.classNames?.title),
          description: cn("text-xs text-muted-foreground", toastOptions?.classNames?.description),
          closeButton: cn(
            "border-border bg-background text-muted-foreground hover:bg-secondary hover:text-foreground",
            toastOptions?.classNames?.closeButton,
          ),
          actionButton: cn(
            "bg-primary text-primary-foreground hover:opacity-90",
            toastOptions?.classNames?.actionButton,
          ),
          cancelButton: cn(
            "bg-secondary text-secondary-foreground hover:bg-secondary/80",
            toastOptions?.classNames?.cancelButton,
          ),
        },
      }}
    />
  )
}
