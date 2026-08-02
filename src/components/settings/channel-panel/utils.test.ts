import { describe, expect, it } from "vitest"
import { readTelegramApiRoot, withTelegramApiRoot } from "./utils"

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
