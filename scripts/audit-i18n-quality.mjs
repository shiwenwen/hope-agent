#!/usr/bin/env node
/**
 * i18n quality audit helper.
 *
 * Examples:
 *   node scripts/audit-i18n-quality.mjs --locales zh,zh-TW --source src/components/settings/memory-panel --fail-on-same
 *   node scripts/audit-i18n-quality.mjs --locales zh-TW --limit 200
 */

import { readFileSync, readdirSync, statSync } from "fs"
import { dirname, relative, resolve } from "path"
import { fileURLToPath } from "url"

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, "..")
const LOCALES_DIR = resolve(ROOT, "src/i18n/locales")

const args = process.argv.slice(2)
const argValue = (name, fallback = null) => {
  const index = args.indexOf(name)
  return index >= 0 && index + 1 < args.length ? args[index + 1] : fallback
}

const localeArg = argValue("--locales", "zh,zh-TW")
const sourceArg = argValue("--source", "src")
const limit = Number(argValue("--limit", "120"))
const failOnSame = args.includes("--fail-on-same")
const auditAll = args.includes("--all")

const locales = localeArg
  .split(",")
  .map((item) => item.trim())
  .filter(Boolean)
const sourceRoot = resolve(ROOT, sourceArg)

// Only values that are language-neutral by definition belong here: product
// names, protocols/acronyms, file names, identifiers, paths, and raw units.
// Translatable UI concepts (Provider, Worktree, Commit, Stage, Model, etc.)
// must not be added merely because they are technical terms.
const ALLOW_SAME_AS_EN = [
  /^Agent$/,
  /^Agents$/,
  /^Sub-Agent$/,
  /^AI Agent$/,
  /^Main Agent$/,
  /^Agent ID$/,
  /^AI$/,
  /^API$/,
  /^API Key$/,
  /^APIKey$/,
  /^API Token$/,
  /^Bot Token$/,
  /^Base URL$/,
  /^App ID$/,
  /^App Secret$/,
  /^Client Secret$/,
  /^Channel Access Token$/,
  /^Channel Secret$/,
  /^Account ID$/,
  /^Chat ID$/,
  /^Thread ID$/,
  /^Hooks$/,
  /^OK$/,
  /^bytes$/,
  /^min$/,
  /^sec$/,
  /^quick_check \{\{value\}\}$/,
  /^\{\{current\}\}\/\{\{total\}\}$/,
  /^\{\{count\}\} http$/,
  /^\{\{n\}\}d$/,
  /^1 min$/,
  /^5 min$/,
  /^PDF$/,
  /^OS$/,
  /^GPU$/,
  /^VRAM$/,
  /^TTFT$/,
  /^JSON$/,
  /^SVG$/,
  /^Mermaid$/,
  /^CSV$/,
  /^XLSX$/,
  /^SHA-256$/,
  /^HTTP$/,
  /^HTTP API$/,
  /^CLI$/,
  /^WebSocket$/,
  /^SQLite$/,
  /^FTS$/,
  /^RRF$/,
  /^MMR$/,
  /^URL$/,
  /^URI$/,
  /^ID$/,
  /^MCP$/,
  /^ACP$/,
  /^YOLO$/,
  /^UTC$/,
  /^Git$/,
  /^GitHub$/,
  /^HEAD$/,
  /^HEAD \{\{hash\}\}$/,
  /^IDE$/,
  /^TLS$/,
  /^USD$/,
  /^ID:$/,
  /^Token$/,
  /^Bluetooth$/,
  /^Retina$/,
  /^Ultra HD$/,
  /^Ultracode$/,
  /^Feishu \/ Lark$/,
  /^Feishu$/,
  /^Google Chat \(Workspace\)$/,
  /^Slack Bot \(Socket Mode\)$/,
  /^WeChat$/,
  /^\+ \.\.\.$/,
  /^\+\{\{count}}$/,
  /^Hope Agent$/,
  /^Hope Agent \{\{from\}\} → \{\{to\}\}$/,
  /^Markdown$/,
  /^HTML$/,
  /^DOCX$/,
  /^MOC$/,
  /^KB$/,
  /^LLM$/,
  /^PAT$/,
  /^Fine-grained PAT$/,
  /^User Agent$/,
  /^AGENTS\.md$/,
  /^IDENTITY\.md$/,
  /^SOUL\.md$/,
  /^TOOLS\.md$/,
  /^Claude$/,
  /^DuckDuckGo$/,
  /^SearXNG$/,
  /^Brave Search$/,
  /^Perplexity$/,
  /^Google Custom Search$/,
  /^Grok \(X\.AI\)$/,
  /^Kimi \(Moonshot\)$/,
  /^Bocha AI Search$/,
  /^(Chutes \(TEE\)|Cloudflare AI Gateway|Google Gemini|Groq|Hugging Face|Kimi Coding|LiteLLM|LM Studio|MiniMax|Mistral|Moonshot AI \(Kimi\)|NVIDIA|NVIDIA AI Endpoints|OpenAI \(Chat\)|OpenRouter|Together AI|vLLM|xAI)$/,
  /^(BytePlus|Baidu Qianfan|Volcengine \(Doubao\)|Xiaomi MiMo|Zhipu AI \(Z\.AI\))$/,
  /^(SiliconFlow \(Qwen-Image \/ Kolors\)|Tongyi Wanxiang \(wanx-v1\)|ZhipuAI \(CogView-4\))$/,
  /^阿里云百炼$/,
  /^(Fal \(Flux\)|Google \(Gemini\)|OpenAI \(DALL-E \/ gpt-image-1\))$/,
  /^IRC \(Internet Relay Chat\)$/,
  /^LINE Messaging API$/,
  /^create_entities\nsearch_nodes$/,
  /^raw\/$/,
  /^sources\/$/,
  /^MOCs\/Conversation Filings\.md$/,
  /^Filed Conversations\/example\.md$/,
  /^folder\/note-name$/,
  /^\{\{size\}\} KB$/,
  /^v\{\{version\}\}$/,
  /^release, ci$/,
  /^ · \{\{error\}\}$/,
  /^(Mem0|Zep|Supermemory|Honcho|Hindsight|OpenViking|OpenAI|Anthropic|Ollama|Gemini|Qwen|DeepSeek|Claude|Chrome|Docker|Discord|Slack|Telegram|WhatsApp|Signal|LINE|iMessage|Google Chat|QQ Bot|IRC)$/,
  /^\{\{[^}]+}}$/,
]

// A handful of established, context-specific product labels intentionally use
// the English term even though the same word is translated elsewhere.
const ALLOW_SAME_KEYS = new Set([
  "settings.localModels.badges.embedding",
  "settings.localModels.filters.embedding",
  "settings.browser.executablePlaceholder",
  "settings.proxyUrlPlaceholder",
  "settings.serverRemoteUrlPlaceholder",
  // Provider concept labels are translated; these values are exact vendor or
  // product names and therefore intentionally remain unchanged.
  "provider_templates.anthropic-vertex.name",
  "provider_templates.fireworks.name",
  "provider_templates.arcee.name",
  "provider_templates.venice.name",
  "provider_templates.synthetic.name",
  "provider_templates.vercel-ai-gateway.name",
  "provider_templates.cerebras.name",
  "provider_templates.deepinfra.name",
  "provider_templates.github-copilot.name",
  "provider_templates.gmi.name",
  "provider_templates.novita.name",
  "provider_templates.opencode.name",
  "provider_templates.opencode-go.name",
  "provider_templates.kilocode.name",
  "provider_templates.sglang.name",
  "provider_templates.tencent.name",
  "provider_templates.stepfun.name",
])

// These values are example file paths, not prose. Translating any path segment
// makes the example invalid for the corresponding feature.
const PROTECTED_EXACT_VALUE_KEYS = new Set([
  "knowledge.queryFile.mocPlaceholder",
  "knowledge.queryFile.createNotePlaceholder",
  "settings.memoryRepairDbSnapshotRestoreQuickCheck",
])

// These values mix translatable prose with machine-facing fragments. Protect
// only the exact fragments that the product or documented command requires.
const PROTECTED_REQUIRED_SEGMENTS_BY_KEY = new Map([
  ["channels.signalHint", ["signal-cli link", "signal-cli register"]],
  ["design.extractHint.url", ["https://example.com"]],
  ["settings.deferredToolsEnabledDesc", ["tool_search"]],
])

const FORBIDDEN_TRANSLATIONS_BY_KEY = new Map([
  ["team.role.lead", new Set(["铅", "鉛", "الرصاص", "plomo", "납", "Kurşun", "Chì"])],
  ["team.memberStatus.idle", new Set(["空闲触发", "閒置觸發"])],
  ["common.statusValues.idle", new Set(["空闲触发", "閒置觸發"])],
  ["common.statusValues.open", new Set(["外部打开", "外部開啟"])],
  ["common.statusValues.resolved", new Set(["已终局", "已終局"])],
  ["common.statusValues.disabled", new Set(["عاجز", "Неполноценный", "Engelli"])],
])

// Catch semantic false friends wherever these source labels are reused. The
// values below describe a person, physical canvas, or adjective instead of the
// software state/action/product label used by the UI.
const FORBIDDEN_TRANSLATIONS_BY_EN_VALUE = new Map([
  ["Clear", new Set(["واضح"])],
  ["Canvas", new Set(["Vải bạt"])],
  ["Disabled", new Set(["عاجز", "Неполноценный", "Engelli"])],
  ["disabled", new Set(["عاجز", "неполноценный", "engelli"])],
])

const ASCII_ELLIPSIS_ALLOWED_KEYS = new Set([
  "channels.slackBotToken",
  "channels.slackAppToken",
  "onboarding.server.apiKeyHint",
  "settings.personaSoulPlaceholder",
  "shortcuts.chordNext",
  "design.draw.scopeEdit",
])
const PASS_SEMANTIC_ENGLISH = /\b(?:pass(?:ed|es|ing)?|fail(?:ed|ing)?|verified|verification|green|clean|blocked)\b/i
const ITEM_SEMANTIC_ENGLISH = /\b(?:items?|matches|clusters?)\b/i

const PLACEHOLDER_BOUNDARY_LOCALES = new Set(["ar", "es", "ms", "pt", "ru", "tr", "vi"])
const CJK_PUNCTUATION_LOCALES = new Set(["zh", "zh-TW"])
const UNEXPECTED_HAN_LOCALES = new Set(["ar", "es", "ko", "ms", "pt", "ru", "tr", "vi"])
const SIMPLIFIED_ONLY_HAN_IN_JAPANESE = /[无输运设这们该为过进显发应态记录认务项话时启览复类约权组处线统级场]/u
const ALLOW_HAN_KEYS = new Set([
  "provider_templates.modelstudio.name",
  "settings.skillsEvolution.fields.sessionRecapThreshold.help",
])
const ALLOW_HAN_EN_KEYS = new Set([
  "channels.pluginDesc_feishu",
  "provider_templates.modelstudio.name",
  "settings.skillsEvolution.fields.sessionRecapThreshold.help",
])
const FOREIGN_SCRIPT_CHECKS = [
  ["Arabic", /\p{Script=Arabic}/u, new Set(["ar"])],
  ["Cyrillic", /\p{Script=Cyrillic}/u, new Set(["ru"])],
  ["Hangul", /\p{Script=Hangul}/u, new Set(["ko"])],
  ["Kana", /[\p{Script=Hiragana}\p{Script=Katakana}]/u, new Set(["ja"])],
]
const COMMAND_LINE = /^ {4}(?:docker|git|pnpm|npm|cargo|hope-agent|gh|curl|node)\b.*$/gm
const INLINE_CODE = /`[^`\n]+`/g
const FENCED_CODE = /```[\s\S]*?```/g
const MARKUP_TAG = /<\/?[A-Za-z][\w:-]*\b[^>]*>/g
const FORMAT_PLACEHOLDER = /\$\{[A-Za-z_][A-Za-z0-9_]*\}|(?<![$\{])\{[A-Za-z_][A-Za-z0-9_]*\}(?!\})/g
const TECHNICAL_IDENTIFIER = /\b[a-z][a-z0-9]*(?:_[a-z0-9]+)+\b/g
const DOTTED_IDENTIFIER = /\b[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+\b/g
const ENV_IDENTIFIER = /\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b/g
const CLI_FLAG = /--[a-z0-9][a-z0-9-]*/gi
const TECHNICAL_FILE = /(?:[A-Za-z0-9_.-]+\/)*[A-Za-z0-9_.-]+\.(?:md|json|ya?ml|toml|tsx?|jsx?|rs|html|csv|xlsx|db)\b/g
const LEGACY_ENGLISH_RESIDUE = [
  /\bCannot Always-Allow\b/,
  /\bCall timeout\b/,
  /\bConfig ready\b/,
  /\bMiniMax M2 series\b/,
  /\bSelect agent\b/,
  /\bgeneration failed\b/i,
  /\bnot run\b/i,
  /\blegacy \/ pending\b/i,
  /\bThe default\b.*\bchanged\b/,
  /\bvector rebuild failed\b/i,
  /\bRetry rebuilding later\b/i,
  /\bShort Global\b/,
]
const BANNED_RAW_UI_LITERALS = new Set([
  "Base URL",
  "API Key",
  "Bot Token",
  "Provider",
  "Worktree",
  "Commit",
  "Stage",
  "Staged",
  "Unstage",
  "Model",
  "Endpoint",
  "Prompt",
  "Sandbox",
  "Workflow",
  "Goal",
  "Plan",
  "Task",
  "Cron",
  "Recap",
  "Artifact",
  "Transport",
  "Profile",
  "Cache",
  "Global",
  "Project",
  "Session",
  "Dashboard",
  "Skill",
  "Tool",
  "Memory",
  "Context",
  "Select agent",
  "Team",
  "Run:",
  "Error",
  "failed",
  "not run",
  "legacy / pending",
])

// Some localized words are legitimately spelled exactly like English. Keep
// these exceptions locale-scoped so they cannot hide a missing translation in
// another language.
const ALLOW_SAME_BY_LOCALE = {
  es: [/^(Auto|Total|Error|Manual|manual|Global|Color|Logo|Avatar|Persona|Formal|Casual|General|Personal|Audio|Visual|Plan|multimodal|Zoom|Local|Base|Web|Total \(ms\))$/],
  ms: [/^(Forum|Auto|Cache|Model|Format|Status|Unit|Domain|Manual|manual|Global|Audio|multimodal|Persona|Avatar|Logo|Bluetooth|Frontmatter|Poster|Video|Video \(MP4\)|Standard|Visual|Grid|Desktop|Tablet|Pen|Trend|Web|Import|Shell)$/],
  pt: [/^(Auto|Total|Status|Manual|manual|Global|Casual|Formal|Zoom|multimodal|Latitude|Longitude|Visual|Local|Base|Link|Total \(ms\))$/],
  ru: [/^Bluetooth$/],
  tr: [/^(Forum|Poster|Video|Video \(MP4\)|Tablet|Bluetooth|Logo|Model|Plan)$/],
  vi: [/^(Web|Video \(MP4\)|Bluetooth)$/],
}

const ALLOW_SAME_KEYS_BY_LOCALE = {
  ar: new Set(["settings.dreaming.title", "settings.memoryTabs.dreaming"]),
  es: new Set([
    "chat.goalCompletion.tokens",
    "dashboard.insights.tokens",
    "dashboard.localModels.installed.badges.embedding",
    "dashboard.session.tokens",
    "dashboard.token.tokensUnit",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
    "workspace.loop.tokensPlaceholder",
    "workspace.workflow.summaryCapTokens",
  ]),
  ko: new Set(["settings.dreaming.title", "settings.memoryTabs.dreaming"]),
  ms: new Set([
    "dashboard.localModels.installed.badges.embedding",
    "knowledge.jobs.model",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
  ]),
  pt: new Set([
    "chat.goalCompletion.tokens",
    "dashboard.insights.tokens",
    "dashboard.localModels.installed.badges.embedding",
    "dashboard.insights.tokensPerMsg",
    "dashboard.session.tokens",
    "dashboard.token.tokensUnit",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
    "workspace.loop.tokensPlaceholder",
  ]),
  ru: new Set([
    "dashboard.localModels.installed.badges.embedding",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
  ]),
  tr: new Set([
    "dashboard.localModels.installed.badges.embedding",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
  ]),
  vi: new Set([
    "canvas.panelTitle",
    "dashboard.localModels.installed.badges.embedding",
    "settings.canvas",
    "settings.dreaming.title",
    "settings.memoryTabs.dreaming",
    "settings.toolCanvasName",
    "tools.canvas",
  ]),
}

// These are ordinary UI concepts, not language-neutral identifiers. Check for
// them even when the rest of the sentence has already been translated; an
// exact-string comparison alone cannot catch mixed-language values such as
// "Todos los Providers".
const UNTRANSLATED_EMBEDDED_TERMS = [
  ["Provider", /\bProviders?\b/i],
  ["Worktree", /\bWorktrees?\b/i],
  ["Commit", /\b(?:Commits?|Committed|Committing)\b/i],
  ["Stage", /\b(?:Stage|Staged|Staging|Unstage|Unstaged)\b/i],
  ["Model", /\bModels?\b/i],
  ["Endpoint", /\bEndpoints?\b/i],
  ["Prompt", /\bPrompts?\b/i],
  ["Sandbox", /\bSandboxes?\b/i],
  ["Workflow", /\bWorkflows?\b/i],
  ["Goal", /\bGoals?\b/i],
  ["Plan", /\bPlans?\b/i],
  ["Task", /\bTasks?\b/i],
  ["Cron", /\bCron\b/i],
  ["Recap", /\bRecaps?\b/i],
  ["Artifact", /\bArtifacts?\b/i],
  ["Transport", /\bTransports?\b/i],
  ["Profile", /\bProfiles?\b/i],
  ["Cache", /\bCaches?\b/i],
  ["Global", /\bGlobal\b/i],
  ["Project", /\bProjects?\b/i],
  ["Session", /\bSessions?\b/i],
  ["Dashboard", /\bDashboards?\b/i],
  ["Skill", /\bSkills?\b/i],
  ["Tool", /\bTools?\b/i],
  ["Memory", /\bMemories?\b/i],
  ["Context", /\bContexts?\b/i],
  ["Team", /\bTeams?\b/i],
  ["Diff", /\bDiffs?\b/i],
  ["Job", /\bJobs?\b/i],
  ["Run", /\bRuns?\b/i],
  ["Default", /\bDefaults?\b/i],
  ["Home", /\bHome\b/i],
  ["Server", /\bServers?\b/i],
]

// A few target languages genuinely use an English-spelled cognate. As with
// exact-value exceptions, keep these scoped to the locale and the single term.
const ALLOW_EMBEDDED_TERMS_BY_LOCALE = {
  es: new Set(["Plan", "Global"]),
  ms: new Set(["Model", "Cache", "Global"]),
  pt: new Set(["Cache", "Global"]),
  tr: new Set(["Model", "Plan"]),
}

// Exact branded phrases may legitimately contain one otherwise-translatable
// term. Keep these exceptions scoped to both the translation key and term.
const ALLOW_EMBEDDED_TERMS_BY_KEY = new Map([
  ["provider_templates.modelstudio.description", new Set(["Model", "Plan"])],
  ["cron.prefixDeliveryWithNameDesc", new Set(["Cron"])],
  ["settings.hooks.scopeNote", new Set(["Project"])],
  ["settings.permissionItems.developer_tools.usage", new Set(["Tool"])],
  ["settings.skillsImport.cc.anthropic", new Set(["Skill"])],
  ["design.deploy.teamId", new Set(["Team"])],
  ["provider.connectRemoteServerDesc", new Set(["Server"])],
  ["slashCommands.permission.description", new Set(["Default"])],
])

function visibleNaturalLanguage(value) {
  return value
    .replace(FENCED_CODE, "")
    .replace(INLINE_CODE, "")
    .replace(MARKUP_TAG, "")
    .replace(/\{\{[^{}]+\}\}/g, "")
    .replace(/https?:\/\/\S+/gi, "")
    .replace(/\bgit\s+commit\b/gi, "")
    .replace(/\bNVIDIA AI Endpoints\b/gi, "")
    .replace(TECHNICAL_IDENTIFIER, "")
    .replace(DOTTED_IDENTIFIER, "")
    .replace(ENV_IDENTIFIER, "")
    .replace(CLI_FLAG, "")
    .replace(TECHNICAL_FILE, "")
    .replace(/\/[a-z][\w-]*/gi, "")
}

function flatKeys(obj, prefix = "") {
  const out = new Map()
  for (const [key, value] of Object.entries(obj)) {
    const full = prefix ? `${prefix}.${key}` : key
    if (value && typeof value === "object" && !Array.isArray(value)) {
      for (const entry of flatKeys(value, full)) out.set(entry[0], entry[1])
    } else {
      out.set(full, value)
    }
  }
  return out
}

function sourceFiles(dir) {
  const files = []
  for (const name of readdirSync(dir)) {
    const full = resolve(dir, name)
    const stat = statSync(full)
    if (stat.isDirectory()) {
      files.push(...sourceFiles(full))
      continue
    }
    if (!/\.(ts|tsx)$/.test(name)) continue
    if (/\.(test|spec)\.(ts|tsx)$/.test(name)) continue
    files.push(full)
  }
  return files
}

function collectProviderTemplateTranslationRefs() {
  const refs = new Map()
  const templateRoot = resolve(
    ROOT,
    "src/components/settings/provider-setup/templates",
  )
  const pattern = /^\s*key:\s*(["'])([^"'\n]+)\1\s*,/gm
  for (const file of sourceFiles(templateRoot)) {
    const source = readFileSync(file, "utf8")
    for (const match of source.matchAll(pattern)) {
      const line = source.slice(0, match.index).split("\n").length
      const location = `${relative(ROOT, file)}:${line}`
      for (const field of ["name", "description"]) {
        refs.set(`provider_templates.${match[2]}.${field}`, [location])
      }
    }
  }
  return refs
}

function collectLiteralTranslationRefs() {
  const refs = new Map()
  // Scan every string literal that exactly matches a locale key. This covers
  // direct t("...") calls as well as lookup tables such as STATUS_I18N_KEYS
  // whose values are passed to t(...) dynamically.
  const pattern = /(["'])([^"'\n]+)\1/g
  for (const file of sourceFiles(sourceRoot)) {
    const source = readFileSync(file, "utf8")
    for (const match of source.matchAll(pattern)) {
      const key = match[2]
      if (!key.includes(".")) continue
      const line = source.slice(0, match.index).split("\n").length
      const rel = relative(ROOT, file)
      if (!refs.has(key)) refs.set(key, [])
      refs.get(key).push(`${rel}:${line}`)
    }
  }
  return refs
}

function collectDirectTranslationRefs() {
  const refs = new Map()
  const patterns = [
    /\b(?:t|i18n\.t)\s*\(\s*(["'])([^"'\n]+)\1/g,
    /<Trans\b[^>]*\bi18nKey\s*=\s*(["'])([^"'\n]+)\1/g,
  ]
  for (const file of sourceFiles(sourceRoot)) {
    const source = readFileSync(file, "utf8")
    for (const pattern of patterns) {
      for (const match of source.matchAll(pattern)) {
        const key = match[2]
        const line = source.slice(0, match.index).split("\n").length
        const rel = relative(ROOT, file)
        if (!refs.has(key)) refs.set(key, [])
        refs.get(key).push(`${rel}:${line}`)
      }
    }
    // Translation lookup tables often store a literal key in one of these
    // fields and call t(item.labelKey) later. Treat them as source references
    // so a missing locale key cannot hide behind defaultValue.
    const indirectPattern =
      /\b(?:labelKey|titleKey|descriptionKey|descKey|errorKey|hintKey|nameKey|promptKey|reasonKey|subjectScopeKey)\s*:\s*(["'])([^"'\n]*\.[^"'\n]*)\1/g
    for (const match of source.matchAll(indirectPattern)) {
      const key = match[2]
      const line = source.slice(0, match.index).split("\n").length
      const rel = relative(ROOT, file)
      if (!refs.has(key)) refs.set(key, [])
      refs.get(key).push(`${rel}:${line}`)
    }
  }
  return refs
}

function collectRawUiLiterals() {
  const hits = []
  const patterns = [
    { pattern: />\s*([A-Za-z][A-Za-z0-9 /:-]{1,60}?)\s*(?=[<{])/g, valueGroup: 1 },
    { pattern: />\s*(Z\.AI \(thinking budget\)|memory #|session)\s*(?=[<{])/g, valueGroup: 1, alwaysReport: true },
    { pattern: /\b(?:placeholder|aria-label|title)\s*=\s*(["'])([^"'\n]+)\1/g, valueGroup: 2 },
    { pattern: /(?:\|\||\?\?)\s*(["'])([^"'\n]+)\1/g, valueGroup: 2 },
  ]
  for (const file of sourceFiles(sourceRoot)) {
    if (!file.endsWith(".tsx")) continue
    const source = readFileSync(file, "utf8")
    for (const { pattern, valueGroup, alwaysReport = false } of patterns) {
      for (const match of source.matchAll(pattern)) {
        const value = match[valueGroup].trim()
        if (!alwaysReport && !BANNED_RAW_UI_LITERALS.has(value)) continue
        const line = source.slice(0, match.index).split("\n").length
        hits.push({ value, location: `${relative(ROOT, file)}:${line}` })
      }
    }
  }
  return hits
}

function protectedSegments(value) {
  const dottedIdentifiers = (value.match(DOTTED_IDENTIFIER) ?? []).filter(
    (segment) => !/^(?:e\.g|i\.e)$/i.test(segment),
  )
  return new Set([
    ...(value.match(COMMAND_LINE) ?? []),
    ...(value.match(INLINE_CODE) ?? []),
    ...(value.match(FENCED_CODE) ?? []),
    ...(value.match(MARKUP_TAG) ?? []),
    ...(value.match(FORMAT_PLACEHOLDER) ?? []),
    ...(value.match(TECHNICAL_IDENTIFIER) ?? []),
    ...dottedIdentifiers,
    ...(value.match(ENV_IDENTIFIER) ?? []),
    ...(value.match(CLI_FLAG) ?? []),
    ...(value.match(TECHNICAL_FILE) ?? []),
  ])
}

function placeholderBoundaryIssues(locale, enValue, localeValue) {
  const issues = []
  const occurrence = new Map()
  for (const match of enValue.matchAll(/\{\{([^{}]+)}}/g)) {
    const list = occurrence.get(match[1]) ?? []
    list.push(match)
    occurrence.set(match[1], list)
  }
  const localeOccurrence = new Map()
  for (const match of localeValue.matchAll(/\{\{([^{}]+)}}/g)) {
    const list = localeOccurrence.get(match[1]) ?? []
    list.push(match)
    localeOccurrence.set(match[1], list)
  }
  const isWord = (char) => !!char && /[\p{L}\p{N}]/u.test(char)
  for (const [name, enMatches] of occurrence) {
    const localeMatches = localeOccurrence.get(name) ?? []
    for (let index = 0; index < Math.min(enMatches.length, localeMatches.length); index += 1) {
      const enMatch = enMatches[index]
      const localeMatch = localeMatches[index]
      const enBefore = enValue[enMatch.index - 1]
      const enAfter = enValue[enMatch.index + enMatch[0].length]
      const localeBefore = localeValue[localeMatch.index - 1]
      const localeAfter = localeValue[localeMatch.index + localeMatch[0].length]
      const localePrefix = localeValue.slice(0, localeMatch.index)
      const attachedArabicParticle = locale === "ar" && /(?:^|[\s،؛])(و|بـ|لـ|كـ)$/.test(localePrefix)
      if (isWord(localeBefore) && !isWord(enBefore) && !attachedArabicParticle) {
        issues.push(`before {{${name}}}`)
      }
      if (isWord(localeAfter) && !isWord(enAfter)) issues.push(`after {{${name}}}`)
    }
  }
  return issues
}

function isAllowedSameAsEnglish(locale, key, value) {
  return (
    key.startsWith("tz.") ||
    key.startsWith("settings.memoryExternalProviderProtocolLabels.") ||
    ALLOW_SAME_KEYS.has(key) ||
    ALLOW_SAME_KEYS_BY_LOCALE[locale]?.has(key) ||
    ALLOW_SAME_AS_EN.some((pattern) => pattern.test(value)) ||
    ALLOW_SAME_BY_LOCALE[locale]?.some((pattern) => pattern.test(value))
  )
}

const en = flatKeys(JSON.parse(readFileSync(resolve(LOCALES_DIR, "en.json"), "utf8")))
const unexpectedHanInEnglish = [...en.entries()].filter(
  ([key, value]) =>
    typeof value === "string" &&
    /[\p{Script=Han}]/u.test(value) &&
    !ALLOW_HAN_EN_KEYS.has(key),
)
const refs = collectLiteralTranslationRefs()
const directRefs = collectDirectTranslationRefs()
const missingDirectRefs = [...directRefs.entries()].filter(([key]) => !en.has(key))
const enRoots = new Set([...en.keys()].map((key) => key.split(".")[0]))
const enPrefixes = new Set()
for (const key of en.keys()) {
  const parts = key.split(".")
  for (let index = 1; index < parts.length; index += 1) {
    enPrefixes.add(parts.slice(0, index).join("."))
  }
}
const translationLikeLiteral =
  /^[a-z][A-Za-z0-9_-]*(?:\.[a-z][A-Za-z0-9_-]*)+$/
const technicalLiteral =
  /\.(?:md|json|ya?ml|toml|tsx?|jsx?|rs|html|css|csv|xlsx?|docx|pptx|db|sqlite|zip|tar|gz|png|jpe?g|gif|svg|webp|bin|m4a|mp3|ogg|wav|webm|mp4)$/i
const missingLikelyLiteralRefs = [...refs.entries()].filter(
  ([key]) =>
    !en.has(key) &&
    translationLikeLiteral.test(key) &&
    enRoots.has(key.split(".")[0]) &&
    !enPrefixes.has(key) &&
    !technicalLiteral.test(key),
)
const missingSourceRefs = new Map([...missingDirectRefs, ...missingLikelyLiteralRefs])
const providerTemplateRefs = collectProviderTemplateTranslationRefs()
const missingProviderTemplateRefs = [...providerTemplateRefs.entries()].filter(
  ([key]) => !en.has(key),
)
const rawUiLiterals = collectRawUiLiterals()
const candidates = auditAll
  ? new Map([...en.keys()].map((key) => [key, refs.get(key) ?? []]))
  : refs
let totalSame = 0
let totalEmbedded = 0
let totalStructural = 0

console.log(`\nsource: ${missingSourceRefs.size} literal/indirect translation keys missing from en`)
for (const [key, locations] of [...missingSourceRefs].slice(0, limit)) {
  console.log(`- ${key}`)
  console.log(`  ${locations[0]}`)
}
if (missingSourceRefs.size > limit) console.log(`... ${missingSourceRefs.size - limit} more`)
console.log(
  `source: ${missingProviderTemplateRefs.length} dynamic Provider template keys missing from en`,
)
for (const [key, locations] of missingProviderTemplateRefs.slice(0, limit)) {
  console.log(`- ${key}`)
  console.log(`  ${locations[0]}`)
}
if (missingProviderTemplateRefs.length > limit) {
  console.log(`... ${missingProviderTemplateRefs.length - limit} more`)
}
console.log(`source: ${rawUiLiterals.length} unlocalized raw UI literals`)
for (const item of rawUiLiterals.slice(0, limit)) {
  console.log(`- ${JSON.stringify(item.value)}`)
  console.log(`  ${item.location}`)
}
if (rawUiLiterals.length > limit) console.log(`... ${rawUiLiterals.length - limit} more`)
console.log(`en: ${unexpectedHanInEnglish.length} unexpected Chinese locale strings`)
for (const [key, value] of unexpectedHanInEnglish.slice(0, limit)) {
  console.log(`- ${key} = ${JSON.stringify(value)}`)
}
if (unexpectedHanInEnglish.length > limit) {
  console.log(`... ${unexpectedHanInEnglish.length - limit} more`)
}

for (const locale of locales) {
  const localeFile = resolve(LOCALES_DIR, `${locale}.json`)
  const data = flatKeys(JSON.parse(readFileSync(localeFile, "utf8")))
  const same = []
  const embedded = []
  const structural = []

  for (const [key, locations] of candidates) {
    const enValue = en.get(key)
    const localeValue = data.get(key)
    if (typeof enValue !== "string") continue
    if (!enValue) continue
    if (localeValue !== enValue) continue
    if (isAllowedSameAsEnglish(locale, key, enValue)) continue
    same.push({ key, value: enValue, locations })
  }

  for (const [key, locations] of candidates) {
    const enValue = en.get(key)
    const localeValue = data.get(key)
    if (typeof enValue !== "string" || typeof localeValue !== "string") continue
    const visibleEnglish = visibleNaturalLanguage(enValue)
    const visibleLocale = visibleNaturalLanguage(localeValue)
    for (const [term, pattern] of UNTRANSLATED_EMBEDDED_TERMS) {
      if (ALLOW_EMBEDDED_TERMS_BY_LOCALE[locale]?.has(term)) continue
      if (ALLOW_EMBEDDED_TERMS_BY_KEY.get(key)?.has(term)) continue
      if (!pattern.test(visibleEnglish) || !pattern.test(visibleLocale)) continue
      embedded.push({ key, term, value: localeValue, locations })
      break
    }
  }

  for (const [key, enValue] of en) {
    const localeValue = data.get(key)
    if (typeof enValue !== "string" || typeof localeValue !== "string") continue

    const protectedExactChanged = PROTECTED_EXACT_VALUE_KEYS.has(key) && localeValue !== enValue
    if (protectedExactChanged) {
      structural.push({ key, issue: "protected exact value changed", value: localeValue })
    }

    if (
      FORBIDDEN_TRANSLATIONS_BY_KEY.get(key)?.has(localeValue) ||
      FORBIDDEN_TRANSLATIONS_BY_EN_VALUE.get(enValue)?.has(localeValue)
    ) {
      structural.push({ key, issue: "known semantic mistranslation", value: localeValue })
    }

    if (
      (locale === "es" || locale === "pt") &&
      /\btokens?\b/i.test(enValue) &&
      /\bfichas?\b/i.test(localeValue)
    ) {
      structural.push({ key, issue: "technical token mistranslated as ficha", value: localeValue })
    }

    for (const segment of PROTECTED_REQUIRED_SEGMENTS_BY_KEY.get(key) ?? []) {
      if (!localeValue.includes(segment)) {
        structural.push({ key, issue: `required technical segment changed: ${JSON.stringify(segment)}`, value: localeValue })
      }
    }

    const enNewlines = enValue.match(/\n/g)?.length ?? 0
    const localeNewlines = localeValue.match(/\n/g)?.length ?? 0
    if (enNewlines !== localeNewlines) {
      structural.push({ key, issue: `newline count ${enNewlines} → ${localeNewlines}`, value: localeValue })
    }

    if (!protectedExactChanged) {
      for (const segment of protectedSegments(enValue)) {
        if (!localeValue.includes(segment)) {
          structural.push({ key, issue: `protected segment changed: ${JSON.stringify(segment)}`, value: localeValue })
        }
      }
    }

    if (PLACEHOLDER_BOUNDARY_LOCALES.has(locale)) {
      for (const issue of placeholderBoundaryIssues(locale, enValue, localeValue)) {
        structural.push({ key, issue: `placeholder joined to text ${issue}`, value: localeValue })
      }
    }

    if (CJK_PUNCTUATION_LOCALES.has(locale)) {
      if (localeValue.includes("...") && !ASCII_ELLIPSIS_ALLOWED_KEYS.has(key)) {
        structural.push({ key, issue: "ASCII ellipsis in localized UI prose", value: localeValue })
      }
      if (/[\p{Script=Han}] [\p{Script=Han}]/u.test(localeValue)) {
        structural.push({ key, issue: "space between Chinese characters", value: localeValue })
      }
      if (/[\p{Script=Han}][,;:]|[,;:](?=[\p{Script=Han}])/u.test(localeValue)) {
        structural.push({ key, issue: "ASCII punctuation next to Chinese text", value: localeValue })
      }
      if (/\([^\n()（）]*）|（[^\n()（）]*\)/u.test(localeValue)) {
        structural.push({ key, issue: "mixed-width parentheses", value: localeValue })
      }
    }


    if (locale === "zh-TW") {
      if (/[“”]/u.test(localeValue)) {
        structural.push({ key, issue: "mainland-style quotation marks in Traditional Chinese", value: localeValue })
      }
      if (localeValue.includes("透過") && PASS_SEMANTIC_ENGLISH.test(enValue)) {
        structural.push({ key, issue: "透過 used for pass/fail semantics", value: localeValue })
      }
      if (localeValue.includes("專案") && ITEM_SEMANTIC_ENGLISH.test(enValue)) {
        structural.push({ key, issue: "專案 used for item/match semantics", value: localeValue })
      }
    }

    if (
      UNEXPECTED_HAN_LOCALES.has(locale) &&
      !ALLOW_HAN_KEYS.has(key) &&
      /[\p{Script=Han}]/u.test(localeValue)
    ) {
      structural.push({ key, issue: "unexpected Chinese text in locale", value: localeValue })
    }

    if (locale === "ja" && SIMPLIFIED_ONLY_HAN_IN_JAPANESE.test(localeValue)) {
      structural.push({ key, issue: "unexpected Simplified Chinese character in Japanese locale", value: localeValue })
    }

    for (const [script, pattern, allowedLocales] of FOREIGN_SCRIPT_CHECKS) {
      if (!allowedLocales.has(locale) && pattern.test(localeValue)) {
        structural.push({ key, issue: `unexpected ${script} text in locale`, value: localeValue })
      }
    }

    for (const pattern of LEGACY_ENGLISH_RESIDUE) {
      if (pattern.test(localeValue)) {
        structural.push({ key, issue: `legacy English residue: ${pattern}`, value: localeValue })
        break
      }
    }
  }

  totalSame += same.length
  totalEmbedded += embedded.length
  totalStructural += structural.length
  const scope = auditAll ? "locale strings" : "source-referenced strings"
  console.log(`\n${locale}: ${same.length} ${scope} equal en`)
  for (const item of same.slice(0, limit)) {
    console.log(`- ${item.key} = ${JSON.stringify(item.value)}`)
    console.log(`  ${item.locations[0] ?? "(not statically referenced)"}`)
  }
  if (same.length > limit) {
    console.log(`... ${same.length - limit} more`)
  }
  console.log(`${locale}: ${embedded.length} untranslated embedded UI terms`)
  for (const item of embedded.slice(0, limit)) {
    console.log(`- ${item.key} [${item.term}] = ${JSON.stringify(item.value)}`)
    console.log(`  ${item.locations[0] ?? "(not statically referenced)"}`)
  }
  if (embedded.length > limit) {
    console.log(`... ${embedded.length - limit} more`)
  }
  console.log(`${locale}: ${structural.length} protected-format issues`)
  for (const item of structural.slice(0, limit)) {
    console.log(`- ${item.key}: ${item.issue}`)
    console.log(`  ${JSON.stringify(item.value)}`)
  }
  if (structural.length > limit) console.log(`... ${structural.length - limit} more`)
}

const blockingIssues =
  missingSourceRefs.size +
  missingProviderTemplateRefs.length +
  rawUiLiterals.length +
  unexpectedHanInEnglish.length +
  totalEmbedded +
  totalStructural

if (blockingIssues > 0 || (failOnSame && totalSame > 0)) {
  process.exit(1)
}
