import { Eye, EyeOff } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

/**
 * Password-style input with an eye-toggle to reveal the value. Used for
 * API keys and other secrets where the user needs the ability to verify
 * what's stored without opening DevTools.
 */
interface SecretInputProps {
  value: string
  onChange: (next: string) => void
  placeholder?: string
  className?: string
  /** Bind a `<label htmlFor>` to the inner input for accessibility. */
  id?: string
}

export function SecretInput({ value, onChange, placeholder, className, id }: SecretInputProps) {
  const [visible, setVisible] = useState(false)
  return (
    <div className={cn("relative", className)}>
      <Input
        id={id}
        type={visible ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="pr-8 font-mono text-xs"
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        onClick={() => setVisible((v) => !v)}
        className="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7 text-muted-foreground hover:text-foreground"
      >
        {visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
      </Button>
    </div>
  )
}
