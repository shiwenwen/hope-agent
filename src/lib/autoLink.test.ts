import { describe, expect, test } from "vitest"
import { findAutoLinkMatches } from "./autoLink"

describe("findAutoLinkMatches", () => {
  test("recognizes raw web links and trims trailing punctuation", () => {
    expect(findAutoLinkMatches("See https://example.com/docs?q=1, please.")).toEqual([
      {
        start: 4,
        end: 32,
        text: "https://example.com/docs?q=1",
        href: "https://example.com/docs?q=1",
      },
    ])
  })

  test("recognizes www links and normalizes href", () => {
    expect(findAutoLinkMatches("Open www.example.com/path")).toEqual([
      {
        start: 5,
        end: 25,
        text: "www.example.com/path",
        href: "https://www.example.com/path",
      },
    ])
  })

  test("recognizes email addresses as mailto links", () => {
    expect(findAutoLinkMatches("Mail dev@example.com")).toEqual([
      {
        start: 5,
        end: 20,
        text: "dev@example.com",
        href: "mailto:dev@example.com",
      },
    ])
  })

  test("does not include an unmatched closing bracket", () => {
    expect(findAutoLinkMatches("(https://example.com/a)")[0]).toMatchObject({
      text: "https://example.com/a",
      href: "https://example.com/a",
    })
  })
})
