export const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const

export type ByteUnit = (typeof BYTE_UNITS)[number]

type FractionDigits =
  | number
  | Partial<Record<ByteUnit, number>>

export interface FormatBytesOptions {
  unit?: ByteUnit
  maxUnit?: ByteUnit
  fractionDigits?: FractionDigits
  trimTrailingZeros?: boolean
}

const UNIT_INDEX: Record<ByteUnit, number> = {
  B: 0,
  KB: 1,
  MB: 2,
  GB: 3,
  TB: 4,
}

const DEFAULT_FRACTION_DIGITS: Record<ByteUnit, number> = {
  B: 0,
  KB: 1,
  MB: 1,
  GB: 1,
  TB: 1,
}

function safeNumber(value: number): number {
  return Number.isFinite(value) ? value : 0
}

function fractionDigitsFor(unit: ByteUnit, fractionDigits?: FractionDigits): number {
  if (typeof fractionDigits === "number") return fractionDigits
  return fractionDigits?.[unit] ?? DEFAULT_FRACTION_DIGITS[unit]
}

function trimTrailingZeros(value: string): string {
  return value.replace(/\.0+$/, "").replace(/(\.\d*?)0+$/, "$1")
}

function formatNumber(value: number, digits: number, trim: boolean): string {
  const formatted = value.toFixed(digits)
  return trim ? trimTrailingZeros(formatted) : formatted
}

export function formatBytes(bytes: number, options: FormatBytesOptions = {}): string {
  const value = safeNumber(bytes)
  const forcedIndex = options.unit ? UNIT_INDEX[options.unit] : undefined
  const maxIndex = options.maxUnit ? UNIT_INDEX[options.maxUnit] : BYTE_UNITS.length - 1
  let unitIndex = forcedIndex ?? 0
  let scaled = value

  if (forcedIndex === undefined) {
    while (Math.abs(scaled) >= 1024 && unitIndex < maxIndex) {
      scaled /= 1024
      unitIndex += 1
    }
  } else {
    scaled = value / 1024 ** forcedIndex
  }

  const unit = BYTE_UNITS[unitIndex]
  const digits = fractionDigitsFor(unit, options.fractionDigits)
  return `${formatNumber(scaled, digits, options.trimTrailingZeros ?? false)} ${unit}`
}

export function formatBytesFromMb(mb: number): string {
  return formatBytes(safeNumber(mb) * 1024 * 1024, {
    maxUnit: "GB",
    fractionDigits: { MB: 0, GB: 1 },
  })
}

export function formatGbFromMb(mb: number): string {
  return formatNumber(safeNumber(mb) / 1024, 1, false)
}
