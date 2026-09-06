import { describe, expect, test } from "vitest"
import { parseAskUserResult } from "./askUserResult"

const choice = {
  questionId: "q",
  question: "Continue?",
  selected: ["Yes"],
  selectedValues: ["yes"],
}

describe("ask-user result compatibility", () => {
  test("legacy timeout selections become fallbacks, never user answers", () => {
    const result = parseAskUserResult(JSON.stringify({ answers: [choice], timedOut: true }))!
    expect(result.answers).toEqual([])
    expect(result.fallback[0].selectedValues).toEqual(["yes"])
    expect(result.timedOut).toBe(true)
  })

  test("supports status-only timeout and legacy text outcomes", () => {
    expect(
      parseAskUserResult(JSON.stringify({ status: "timed_out", answers: [], fallback: [choice] }))
        ?.fallback,
    ).toHaveLength(1)
    expect(
      parseAskUserResult(
        "The questions timed out after 60 seconds without a response and no default values were provided.",
      )?.timedOut,
    ).toBe(true)
    expect(
      parseAskUserResult("The user cancelled the questions without answering.")?.cancelled,
    ).toBe(true)
  })

  test("real answers retain option identity and ignore fallback data", () => {
    const result = parseAskUserResult(
      JSON.stringify({ status: "answered", answers: [choice], fallback: [choice] }),
    )!
    expect(result.answers[0].questionId).toBe("q")
    expect(result.answers[0].selectedValues).toEqual(["yes"])
    expect(result.fallback).toEqual([])
  })

  test("malformed entries and cancelled payloads cannot appear as answers", () => {
    const result = parseAskUserResult(JSON.stringify({ status: "cancelled", answers: [choice] }))!
    expect(result.answers).toEqual([])
    expect(result.cancelled).toBe(true)
    expect(
      parseAskUserResult(
        JSON.stringify({ answers: [null, {}, { question: "x", selected: "bad" }] }),
      )?.answers,
    ).toEqual([])
    expect(parseAskUserResult("Error: invalid timeout")).toBeNull()
  })
})
