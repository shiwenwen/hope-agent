interface CacheUsageDisplayOptions {
  created: number
  read: number
  writeLabel: string
  hitLabel: string
}

export function formatCompactTokenCount(count: number): string {
  return count > 1000 ? `${(count / 1000).toFixed(1)}k` : String(count)
}

export function formatCacheUsageDisplay(options: CacheUsageDisplayOptions): string {
  const created = formatCompactTokenCount(options.created)
  const read = formatCompactTokenCount(options.read)
  return `${options.writeLabel} ${created} / ⚡${options.hitLabel} ${read}`
}
