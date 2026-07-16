import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"
import { describe, expect, it } from "vitest"

const SRC_ROOT = resolve(process.cwd(), "src")
const SOURCE_FILE = /\.(?:css|ts|tsx)$/
const TEST_FILE = /\.(?:test|spec)\.(?:ts|tsx)$/
const HOVER_BORDER_UTILITY = /(?:hover|group-hover|peer-hover):(?:border|ring)(?:-|\b)/g

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return sourceFiles(path)
    if (!SOURCE_FILE.test(entry.name) || TEST_FILE.test(entry.name)) return []
    return [path]
  })
}

describe("interaction border audit", () => {
  it("keeps hover feedback on backgrounds instead of borders or rings", () => {
    const violations: string[] = []

    for (const file of sourceFiles(SRC_ROOT)) {
      const source = readFileSync(file, "utf8")
      for (const match of source.matchAll(HOVER_BORDER_UTILITY)) {
        const line = source.slice(0, match.index).split("\n").length
        violations.push(`${relative(process.cwd(), file)}:${line} ${match[0]}`)
      }
    }

    expect(violations).toEqual([])
  })
})
