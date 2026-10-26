import { describe, expect, it } from "vitest"
import { readTelegramApiRoot, withTelegramApiRoot, readWeComGroups, withWeComGroups } from "./utils"

describe("Telegram API root settings", () => {
  it("reads and trims an existing API root", () => {
    expect(readTelegramApiRoot({ apiRoot: " https://tg.example.com/ " })).toBe(
      "https://tg.example.com/",
    )
    expect(readTelegramApiRoot(null)).toBe("")
  })

  it("updates without mutating unrelated channel settings", () => {
    const original = { proxy: "socks5://127.0.0.1:1080", imReplyMode: "split" }
    const updated = withTelegramApiRoot(original, " https://tg.example.com/api ")

    expect(updated).toEqual({
      proxy: "socks5://127.0.0.1:1080",
      imReplyMode: "split",
      apiRoot: "https://tg.example.com/api",
    })
    expect(original).not.toHaveProperty("apiRoot")
  })

  it("removes the key when the official endpoint is selected", () => {
    expect(withTelegramApiRoot({ apiRoot: "https://tg.example.com", proxy: "proxy" }, " ")).toEqual(
      { proxy: "proxy" },
    )
  })
})

describe("WeCom group registration", () => {
  it("preserves existing strict policies and only opts newly named groups into unmentioned text", () => {
    const strict = {
      requireMention: true,
      enabled: false,
      allowFrom: ["owner"],
      agentId: "private-agent",
      topics: {},
    }
    const previous = { "group:old": strict, "*": strict }
    expect(withWeComGroups(previous, readWeComGroups(previous))).toEqual(previous)
    const next = withWeComGroups(previous, "group:old\nnew\nnew")
    expect(next["group:old"]).toBe(strict)
    expect(next["group:new"]).toEqual({
      requireMention: false,
      enabled: true,
      allowFrom: [],
      topics: {},
    })
    expect(previous).not.toHaveProperty("group:new")
  })
  it("does not turn empty or wildcard input into open groups or interpret prototype names as configuration", () => {
    expect(withWeComGroups({}, "\n, *\ngroup:*\ngroup:")).toEqual({})
    const next = withWeComGroups({}, "__proto__\nconstructor")
    expect(Object.keys(next)).toEqual(["group:__proto__", "group:constructor"])
    expect(Object.getPrototypeOf(next)).toBe(Object.prototype)
  })
})
