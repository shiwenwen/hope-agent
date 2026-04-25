import { readFileSync, readdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { test, expect } from "vitest"

const localesDir = join(dirname(fileURLToPath(import.meta.url)), "locales")

const expectedPlaceholders: Record<string, string> = {
  "ar.json": "مثال: الرد دائمًا بالعربية؛ اجعل النبرة واضحة ولطيفة؛ ابدأ بالخلاصة",
  "en.json": "e.g. Always reply in English; keep the tone clear and friendly; start with the conclusion",
  "es.json": "p. ej. Responde siempre en español; usa un tono claro y amable; empieza por la conclusión",
  "ja.json": "例：常に日本語で返答する。明るく親しみやすい口調にし、最初に結論を述べる",
  "ko.json": "예: 항상 한국어로 응답; 명확하고 친근한 톤 유지; 결론부터 말하기",
  "ms.json": "cth. Sentiasa balas dalam BM; gunakan nada jelas dan mesra; mulakan dengan kesimpulan",
  "pt.json": "ex. Sempre responder em português; usar um tom claro e amigável; começar pela conclusão",
  "ru.json": "напр. Всегда отвечай на русском; сохраняй ясный и дружелюбный тон; начинай с вывода",
  "tr.json": "örn. Her zaman Türkçe yanıtla; tonu açık ve samimi tut; sonuca önce başla",
  "vi.json": "vd. Luôn trả lời bằng tiếng Việt; giữ giọng rõ ràng và thân thiện; bắt đầu bằng kết luận",
  "zh-TW.json": "例如：始終用繁體中文回覆；語氣清楚親切；先給結論再補充細節",
  "zh.json": "例如：始终用中文回复；语气清楚亲切；先给结论再补充细节",
}

test("project instruction placeholders use non-technical examples in every locale", () => {
  const localeFiles = readdirSync(localesDir)
    .filter((file) => file.endsWith(".json"))
    .sort()

  expect(Object.keys(expectedPlaceholders).sort()).toEqual(localeFiles)

  for (const file of localeFiles) {
    const locale = JSON.parse(readFileSync(join(localesDir, file), "utf8")) as {
      project?: { projectInstructionsPlaceholder?: string }
    }
    const placeholder = locale.project?.projectInstructionsPlaceholder

    expect(placeholder, file).toBe(expectedPlaceholders[file])
    expect(placeholder ?? "", file).not.toMatch(/Tauri|React|stack|技术栈|技術棧/i)
  }
})
