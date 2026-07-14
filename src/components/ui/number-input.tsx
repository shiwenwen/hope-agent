import * as React from "react"

import { Input, type InputProps } from "@/components/ui/input"

export type NumberInputProps = Omit<InputProps, "type">

/** Unified numeric input surface that preserves native number semantics. */
const NumberInput = React.forwardRef<HTMLInputElement, NumberInputProps>((props, ref) => (
  <Input ref={ref} type="number" {...props} />
))

NumberInput.displayName = "NumberInput"

export { NumberInput }
