import { expect, test } from "vitest"

import { manualSlug } from "./manualSlug"

// Ground-truth pairs from real anchors in docs/user-guide — the SAME cases
// asserted by the Rust test `slugs_match_github_anchors_from_the_real_docs`
// (crates/ha-core/src/manual/model.rs). If either side drifts, anchors break.
test("matches GitHub anchors from the real docs (shared fixture with Rust)", () => {
  const cases: Array<[string, string]> = [
    ["4.1 三层记忆：全局 / Agent / 项目", "41-三层记忆全局--agent--项目"],
    ["7.8 电脑控制（macOS）", "78-电脑控制macos"],
    ["2.11 语音转写(STT)", "211-语音转写stt"],
    ["2.4 Sign in with a ChatGPT / Codex account", "24-sign-in-with-a-chatgpt--codex-account"],
    ["13.1 设置界面导航地图", "131-设置界面导航地图"],
    ["Core concepts (all in one place)", "core-concepts-all-in-one-place"],
  ]
  for (const [heading, anchor] of cases) {
    expect(manualSlug(heading), heading).toBe(anchor)
  }
})

