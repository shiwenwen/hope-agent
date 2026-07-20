import { expect, test } from "vitest"

import { resolveManualLink, resolveRenderedHref, rewriteManualBody } from "./helpLinks"

test("same-chapter anchors", () => {
  expect(resolveManualLink("#41-三层记忆全局--agent--项目", "zh")).toEqual({
    kind: "anchor",
    anchor: "41-三层记忆全局--agent--项目",
  })
  expect(resolveManualLink("#", "zh")).toEqual({ kind: "none" })
})

test("cross-chapter links with and without anchors, both languages", () => {
  expect(resolveManualLink("05-知识空间.md", "zh")).toEqual({ kind: "chapter", chapter: 5 })
  expect(resolveManualLink("08-自主任务.md#85-计划模式plan-mode", "zh")).toEqual({
    kind: "chapter",
    chapter: 8,
    anchor: "85-计划模式plan-mode",
  })
  expect(resolveManualLink("02-models-and-providers.md", "en")).toEqual({
    kind: "chapter",
    chapter: 2,
  })
  expect(resolveManualLink("./03-对话与会话.md", "zh")).toEqual({ kind: "chapter", chapter: 3 })
})

test("README goes back to the index chapter", () => {
  expect(resolveManualLink("README.md", "zh")).toEqual({ kind: "chapter", chapter: 0 })
})

test("language switch links from the two READMEs", () => {
  expect(resolveManualLink("en/README.md", "zh")).toEqual({ kind: "language-switch" })
  expect(resolveManualLink("../README.md", "en")).toEqual({ kind: "language-switch" })
})

test("external http(s) links pass through", () => {
  expect(resolveManualLink("https://ollama.com/download", "zh")).toEqual({
    kind: "external",
    url: "https://ollama.com/download",
  })
})

test("links escaping the manual resolve to GitHub, per-language depth", () => {
  expect(resolveManualLink("../deployment/docker.md", "zh")).toEqual({
    kind: "external",
    url: "https://github.com/shiwenwen/hope-agent/tree/main/docs/deployment/docker.md",
  })
  expect(resolveManualLink("../../deployment/docker.md", "en")).toEqual({
    kind: "external",
    url: "https://github.com/shiwenwen/hope-agent/tree/main/docs/deployment/docker.md",
  })
  expect(resolveManualLink("../../README.md", "zh")).toEqual({
    kind: "external",
    url: "https://github.com/shiwenwen/hope-agent/tree/main/README.md",
  })
  expect(resolveManualLink("../../../README.en.md", "en")).toEqual({
    kind: "external",
    url: "https://github.com/shiwenwen/hope-agent/tree/main/README.en.md",
  })
  expect(resolveManualLink("../architecture/", "zh")).toEqual({
    kind: "external",
    url: "https://github.com/shiwenwen/hope-agent/tree/main/docs/architecture",
  })
})

test("unrecognized or escaping-the-repo links never navigate", () => {
  expect(resolveManualLink("../../../../etc/passwd", "zh")).toEqual({ kind: "none" })
  expect(resolveManualLink("/absolute/path.md", "zh")).toEqual({ kind: "none" })
  expect(resolveManualLink("", "zh")).toEqual({ kind: "none" })
})

// rehype-harden (no defaultOrigin) blocks bare relative hrefs, so every
// relative manual link must be rewritten to a fragment or absolute URL
// BEFORE rendering — this rewrite is what keeps chapter navigation alive.
test("rewriteManualBody turns relative manual links into harden-safe targets", () => {
  const zh = rewriteManualBody(
    "见 [02](02-模型与Provider.md#锚点) 与 [05](05-知识空间.md)、[目录](README.md)、" +
      "[English](en/README.md)、[docker](../deployment/docker.md)、" +
      "[页内](#41-三层记忆)、[外](https://ollama.com/download)",
    "zh",
  )
  expect(zh).toContain("](#ch:2:锚点)")
  expect(zh).toContain("](#ch:5)")
  expect(zh).toContain("](#ch:0)")
  expect(zh).toContain("](#lang-switch)")
  expect(zh).toContain(
    "](https://github.com/shiwenwen/hope-agent/tree/main/docs/deployment/docker.md)",
  )
  // Fragments and absolute URLs pass through untouched.
  expect(zh).toContain("](#41-三层记忆)")
  expect(zh).toContain("](https://ollama.com/download)")
  // en README's language switcher resolves at en depth.
  expect(rewriteManualBody("[简体中文](../README.md)", "en")).toBe("[简体中文](#lang-switch)")
})

test("resolveRenderedHref round-trips the rewritten targets", () => {
  expect(resolveRenderedHref("#ch:8:85-计划模式plan-mode")).toEqual({
    kind: "chapter",
    chapter: 8,
    anchor: "85-计划模式plan-mode",
  })
  expect(resolveRenderedHref("#ch:0")).toEqual({ kind: "chapter", chapter: 0 })
  expect(resolveRenderedHref("#lang-switch")).toEqual({ kind: "language-switch" })
  expect(resolveRenderedHref("#41-三层记忆")).toEqual({ kind: "anchor", anchor: "41-三层记忆" })
  expect(resolveRenderedHref("https://example.com")).toEqual({
    kind: "external",
    url: "https://example.com",
  })
  expect(resolveRenderedHref("")).toEqual({ kind: "none" })
  expect(resolveRenderedHref("05-知识空间.md")).toEqual({ kind: "none" })
})
