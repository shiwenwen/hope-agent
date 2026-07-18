# 02 · Models & Providers

This chapter explains how to get the AI working with a model: connecting a model service, filling in an API key or signing in with an account, choosing your primary model and fallback models, tuning thinking effort, and configuring companion services such as local models, speech-to-text, AI image generation, and web search.

**In this chapter**

- [2.1 Connecting a model service (Provider)](#21-connecting-a-model-service-provider)
- [2.2 Multiple API keys per provider (automatic rotation)](#22-multiple-api-keys-per-provider-automatic-rotation)
- [2.3 Four API types](#23-four-api-types)
- [2.4 Sign in with a ChatGPT / Codex account](#24-sign-in-with-a-chatgpt--codex-account)
- [2.5 Choosing the primary model and fallback chain](#25-choosing-the-primary-model-and-fallback-chain)
- [2.6 Temperature and thinking effort](#26-temperature-and-thinking-effort)
- [2.7 Automatic failover](#27-automatic-failover)
- [2.8 Background models and vision model](#28-background-models-and-vision-model)
- [2.9 One-click local model install](#29-one-click-local-model-install)
- [2.10 Memory embedding model](#210-memory-embedding-model)
- [2.11 Speech-to-Text (STT)](#211-speech-to-text-stt)
- [2.12 AI image and audio generation](#212-ai-image-and-audio-generation)
- [2.13 Web search and web fetch](#213-web-search-and-web-fetch)

> Most users only need to connect a single provider via [2.1](#21-connecting-a-model-service-provider) to get started. Consult the other sections as needed.

---

## 2.1 Connecting a model service (Provider)

Hope Agent connects to each vendor's large models through a "**provider + API key**" pairing. It ships with **40+ provider templates and 300+ preset models**, and also supports any custom OpenAI-compatible or Anthropic endpoint.

**Where**: Settings → **Model Configuration** → **Providers** tab → click "**Add Provider**" in the top right.

The first page of the add wizard offers five paths:

1. **Sign in with ChatGPT (Codex)** — the primary button at the top; sign in with an account instead of an API key. See [2.4](#24-sign-in-with-a-chatgpt--codex-account).
2. **Connect to a remote server** — connect to a machine already running a Hope Agent service and reuse its configuration. See [01 · Access from your phone or another computer](01-getting-started.md#14-access-from-your-phone-or-another-computer).
3. **Local model assistant** — appears when Ollama is not installed; installs a local model in one click. See [2.9](#29-one-click-local-model-install).
4. **Choose a built-in template** — a searchable grid of providers; those already configured get a green "Configured" badge.
5. **Custom provider** — connect to any compatible endpoint.

### Using a built-in template (recommended)

Click a template (such as Anthropic, OpenAI, DeepSeek, or Tongyi Qianwen) to open its configuration page and fill in the **API key** (for some local / unauthenticated endpoints the key is marked "optional"). You can click "Test connection" to verify, then "Done" to save. The template has already pre-filled the endpoint address, API type, thinking style, and model list — usually you don't need to change anything.

After you add a provider, the system automatically sets its first model as the current default model, so you can start chatting right away.

### Custom provider

If your provider has no ready-made template, choose "Custom provider" and follow the three-step wizard:

1. Pick the **API type** (Anthropic / OpenAI Chat / OpenAI Responses; see [2.3](#23-four-api-types)); if unsure, choose **OpenAI Chat** for the best compatibility.
2. Fill in the provider name, **Base URL**, API key (optional), and thinking style; you can test the connection.
3. Add models manually: model ID, display name, supported input types (text / image / video), context window, max output, whether it is a reasoning model, and unit price.

### Common provider settings

In the provider list you can drag to reorder, enable / disable, edit, and delete (via the "⋮" menu on the right). The edit page also has these fields:

| Setting | Default | What it does |
| --- | --- | --- |
| Billing currency | US dollar (USD) | The currency of the model's unit price (USD / CNY); affects only how cost statistics are converted. Built-in providers such as Tongyi / Volcano / Tencent are already marked as CNY |
| Allow access to private networks | Off | Appears only when the Base URL is an intranet address (e.g. local Ollama / LM Studio); enabling it allows localhost and adds that host to the security allowlist |
| User-Agent | `claude-code/0.1.0` | The UA in the request headers; usually no need to change |
| Thinking style | Template value | Determines how reasoning parameters are sent; see [2.6](#26-temperature-and-thinking-effort) |
| Model list | Template preset | Each model can be configured with an ID, display name, input types, context window, max output, whether it is a reasoning model, and unit price |

> **About API key security**: The desktop GUI is a trusted local environment, so the key field is a password box with an "eye" icon that lets you view the plaintext locally; leaving it blank when editing keeps the existing key. **However, the provider list and API keys cannot be modified by the AI through conversation** — this is a deliberate security design, and the AI sees masked values when reading them. See [13 · Settings & Security](13-settings-and-security.md).

---

## 2.2 Multiple API keys per provider (automatic rotation)

Attach multiple API keys to the same provider, and when one is rate-limited or fails, it will **automatically switch to the next**, balancing usage and improving resilience against rate limits.

**Where**: the "API Key Rotation Profiles" section on the **Edit an existing provider** page. Note — the initial add wizard only fills in a single key; to add multiple keys you must save first, then come back to edit the provider.

Each key configuration includes: a label (to tell them apart), the API key, an enable toggle, and an optional dedicated Base URL.

**How it works**: on rate-limit, overload, authentication, or billing errors it automatically switches keys and gives the failed key a cooldown period; for errors like network timeouts where "switching wouldn't help," it only retries the same key. A given session tries to "stick" to the same key to hit the model's prompt cache and save costs.

> Providers signed in with a ChatGPT / Codex account do not participate in key rotation (there is no rotatable key). See [2.4](#24-sign-in-with-a-chatgpt--codex-account).

---

## 2.3 Four API types

The API type tells Hope Agent which protocol to use when talking to the provider. **When using a built-in template it is already pre-filled, so you don't need to worry about it**; you only need to pick the right one according to the provider's documentation for custom endpoints.

| Type | Description | When to use |
| --- | --- | --- |
| **OpenAI Chat** | The most universal `/v1/chat/completions` protocol | The vast majority of third-party / domestic / aggregator providers (DeepSeek, Tongyi, Zhipu, Moonshot, Volcano, OpenRouter, local Ollama, etc.). **The first choice for custom endpoints** |
| **Anthropic** | Claude's official Messages API | Anthropic and services that adopt its protocol |
| **OpenAI Responses** | OpenAI's new `/v1/responses` protocol | OpenAI official (default), GitHub Copilot |
| **Codex** | Sign in with a ChatGPT account instead of an API key | See [2.4](#24-sign-in-with-a-chatgpt--codex-account); **cannot be selected manually, only produced by signing in** |

When automatically switching models across protocols (for example, when the primary model fails and falls back to another vendor), the system automatically converts the message format for you — you don't need to worry about it.

---

## 2.4 Sign in with a ChatGPT / Codex account

Don't want to enter an API key by hand? You can sign in directly with a ChatGPT account, use the GPT-5.x series as your primary chat model, and consume your subscription quota.

**Sign in**: Settings → Add Provider → the "**Sign in with ChatGPT (Codex)**" primary button at the top of the first page opens the system browser to complete authorization (finish within 5 minutes). Once signed in, the configuration is saved automatically and a green "Configured" badge appears.

You can also sign in from the command line (suitable for remote / headless machines):

```bash
hope-agent auth codex login          # Sign in on this machine
hope-agent auth codex login --no-open # Print the authorization link only (pair with SSH port forwarding)
hope-agent auth codex status         # Check sign-in status
hope-agent auth codex logout         # Sign out (deletes the Codex provider and local credentials)
```

**Re-sign in**: the Codex provider card's "⋮" menu has "Re-sign in" (rather than "Delete", to avoid accidental removal); when the token expires the interface also guides you to sign in again.

- The specific model used after signing in can be switched in Settings; the default is the tier available across all account levels.
- Codex credentials are stored only on this machine (`~/.hope-agent/credentials/auth.json`) and are never written to logs.
- Codex sign-in and the account sign-in for [11 · MCP: connecting external tools](11-connect-and-extend.md#111-mcp-connecting-external-tools) are two completely independent mechanisms that do not affect each other.

---

## 2.5 Choosing the primary model and fallback chain

**Where**: Settings → Model Configuration → **Global Model** tab.

- **Default model**: a two-level dropdown (pick the provider first, then the model); this is the model all new sessions use by default.
- **Fallback chain (Fallback)**: you can add multiple and drag to reorder; when the primary model call fails, the system automatically tries the next one in this order. See [2.7](#27-automatic-failover).

**A few key points**:

- A session "pins" the model at the time it is created; changing the global default afterward **does not affect existing sessions**, only new ones. To temporarily swap the model for a single session, use the `/model` command in the conversation or the model entry in the input box.
- Each Agent can also configure its own model chain; if an Agent has none configured, it follows the global setting.
- The fallback chain is only a temporary downgrade for the current round; the next round still starts fresh from the primary model.

---

## 2.6 Temperature and thinking effort

Two knobs that affect answer style, both layered as "**session > Agent > global**" overrides (the session level has the highest priority).

**Where**: Settings → Global Model.

- **Thinking effort (Think)**: controls how deeply the model thinks, with values up to `none / minimal / low / medium / high / xhigh` (the actual available tiers depend on the model's API type; for example Claude / OpenAI Chat only offer `none / low / medium / high`). The global default is `medium`. In a conversation you can use `/thinking high` to adjust it quickly.
- **Temperature**: controls the randomness of answers, a slider from 0.00–2.00, defaulting to 1.00. A "Reset" button returns it to the inherited state.

**Thinking Style** is a provider-level setting (which can also be overridden per model) that determines "how" reasoning parameters are sent. Built-in templates have it pre-filled, so usually you don't need to touch it. There are 5 styles:

| Style | Applies to |
| --- | --- |
| OpenAI (`reasoning_effort`) | OpenAI / DeepSeek / Mistral / xAI, etc. |
| Anthropic (`thinking budget_tokens`) | Claude, MiniMax, Kimi, etc. |
| Zai | Zhipu Z.AI |
| Qwen (`enable_thinking`) | Tongyi Qianwen / Alibaba Cloud Bailian |
| None | Sends no thinking parameters (some custom endpoints) |

> If a model is not a reasoning model itself, the system automatically sends no thinking parameters; you don't need to distinguish this by hand.

---

## 2.7 Automatic failover

This is a built-in capability with **no separate switch**; it takes effect automatically along with your model chain ([2.5](#25-choosing-the-primary-model-and-fallback-chain)) and multiple keys ([2.2](#22-multiple-api-keys-per-provider-automatic-rotation)). Whenever a model call fails, the system automatically determines the error type and handles it, largely invisibly to you; occasionally you'll see a "model downgraded" hint in the conversation.

The handling strategy varies by scenario:

- **Main conversation**: maximum retries, key switching allowed, and switching to the next model on failure (the strongest resilience).
- **Background tasks** (such as generating titles or extracting memories): light retries (so you don't have to wait long).
- **Context summarization**: fast fail-and-downgrade (you're waiting for a reply, so speed wins).

How common errors are handled: rate-limit / overload → backoff retry + key switch; network timeout → retry only; context too long → auto-compact then retry; authentication failure → key switch (for Codex, prompts you to re-sign in); model not found → switch straight to the next fallback model.

> **Codex signs in with an account and has no multiple keys to rotate**, so once it hits an authentication failure it goes straight to trying the next model in the chain and prompts you to re-sign in to Codex.

---

## 2.8 Background models and vision model

At the bottom of the "Global Model" panel, you can designate dedicated models for two special purposes, independent of the main chat model:

- **Vision bridge model (Vision)** — when your primary model can't understand images (doesn't support vision) but receives one, this model converts the image into a text description that is injected into the conversation, instead of "dropping the image." **Off by default**; you must manually designate an image-capable model to enable it. The transcribed text is treated as "untrusted external data" and is never taken as an instruction to the AI.
- **Automation default model chain (Automation)** — designates a unified default model chain for background tasks that "only need a single model call, not a full AI persona" (generating session titles, Recap reports, memory consolidation, Knowledge Space maintenance, skill review, etc.). If a given feature panel sets its own model, that takes priority; otherwise the global default here is used; failing that, it follows the main chat model.

Both are pure model references that carry no credentials, and their risk level is medium (MEDIUM).

---

## 2.9 One-click local model install

No account, API key, or terminal required — pick a local model that suits your hardware in Settings, and the app handles the whole flow of **installing Ollama → downloading the model → registering the provider → setting it as default**. The data stays entirely local and works without an internet connection.

### The easy way: the local model assistant card

**Location**: the model settings page, or the Provider step of the first-run wizard.

The card automatically recommends a model based on your hardware (showing size, context window, and whether it is a reasoning model); you can "Expand alternatives" to switch to other candidates. A single primary button adapts to Ollama's status:

- Ollama not installed → "Install Ollama" (auto-installs on macOS / Linux; on Windows it guides you to download from [ollama.com/download](https://ollama.com/download)).
- Installed but not running / recommended model not downloaded → "Install model".
- All ready → shows "Enabled".

When done, it **automatically** adds the model to the provider list and sets it as the global default. The install / download process has a progress dialog (stage, bytes, estimated time remaining, logs), which can be backgrounded or cancelled.

### Explicit management: the Local Models tab

The status area at the top shows Ollama's status (not installed / installed / running). In the list of installed models, each model can be: started / stopped (loaded into memory or unloaded), added to the provider list, set as default, added to the Embedding configuration, deleted, and more. You can also search the Ollama model library and enter a name manually to download.

> **Recommendation basis**: the budget takes 60% of macOS unified memory, or 60% of Windows/Linux discrete-GPU VRAM (falling back to 60% of system memory if insufficient), then subtracts about 1 GiB of runtime overhead, and picks the first model that fits within the budget, from largest to smallest.
>
> **The app does not take over the Ollama process**: it only tries to start Ollama when needed, and **does not kill Ollama** when the app exits, so it won't affect your other tools that use Ollama. Before deleting a model, it lists all references and asks you to confirm.

---

## 2.10 Memory embedding model

Configuring an Embedding (vector) model lets [memory](04-memory.md) and the [Knowledge Space](05-knowledge-space.md) perform **semantic search** (finding by meaning, not just by keyword). Without one, it degrades to pure keyword search.

**Where**: Settings → Memory / Embedding model panel. Three ways to configure:

1. **Local quick card**: install Ollama + download a recommended embedding model in one click, automatically set as default and rebuild vectors.
2. **Quick-create from template**: OpenAI / Google Gemini / Jina / Cohere / SiliconFlow / Voyage / Mistral / Ollama.
3. **Custom**: fill in the provider type, Base URL, API key, model name, and dimensions.

> **Switching the default Embedding model triggers a full vector rebuild (reembed)** and requires a second confirmation — because different models' vector spaces are not interchangeable. Memory and the Knowledge Space share the same "model library" but each independently chooses which to use.
>
> The Embedding model choice **can only be changed in the GUI**, not modified by the AI through conversation (it carries an API key, and switching models has the heavy side effect of rebuilding vectors).

---

## 2.11 Speech-to-Text (STT)

A "speech-to-text" engine independent of the main model, used for: desktop microphone input (with live transcription as you speak) and automatic transcription of voice messages received over IM channels.

**Where**: Settings → **Speech-to-Text** panel.

- **Primary model**: the preferred transcription model for desktop voice input (streaming models are marked `streaming` and produce text as you speak).
- **IM fallback model**: dedicated to IM auto-transcription (only non-streaming models can be selected); if unset, the primary model is used.
- **Providers**: "Add Provider" supports 10 protocols including OpenAI, Groq, ElevenLabs, xAI, DashScope (Bailian), Deepgram, AssemblyAI, Azure, iFlytek, and Volcano; local backends (whisper.cpp / faster-whisper / FunASR / sherpa-onnx) can be detected and connected in one click.

**How to use desktop recording**: click the microphone button in the chat input box to record; you can also hold `Ctrl+Shift+H` within the app window to speak, release to transcribe, and the text is inserted at the cursor. During recording, a waveform and real-time elapsed time are shown.

**IM auto-transcription**: off by default; the toggle is in the edit dialog of **each IM channel account** (not in the Speech-to-Text panel). Once enabled, voice received in IM is automatically converted to text (prefixed with `[Voice transcription]`) and injected into the conversation.

> The speech provider list and keys can likewise **only be changed in the GUI**; the AI can only read a masked summary. The only thing you can adjust through conversation is the "IM auto-transcription" toggle. The per-audio limit is 25 MiB.

---

## 2.12 AI image and audio generation

### AI image generation (image_generate)

Connect text-to-image / image-editing providers for the `image_generate` tool in conversations and for the [Design Space](06-design-space.md).

**Where**: Settings → **Image Generation** panel. Supports 7 vendors: OpenAI (gpt-image), Google (Gemini / Imagen, supporting image editing and multiple reference images), Fal (Flux), MiniMax, SiliconFlow (Qwen-Image), Zhipu (CogView), and Tongyi Wanxiang. Each can be enabled, given a key, have a model selected, and be tested for connectivity; cards can be dragged to reorder (the order is the priority for failover).

| Setting | Default | What it does |
| --- | --- | --- |
| Default size | 1024×1024 | The output size when unspecified |
| Request timeout | 60 seconds | Timeout for a single generation (10–300) |
| Google image reasoning depth | MINIMAL | The reasoning tier for Gemini image generation |

When a reference image is present, some providers automatically switch to the corresponding image-editing endpoint. The API key in the panel is a password box; the AI sees it masked when reading it through conversation.

### Audio generation (audio_generate)

Connect TTS / music / sound-effect providers, primarily serving the "audio" artifacts of the [Design Space](06-design-space.md).

**Where**: Settings → **Audio Generation** panel. Supports OpenAI (TTS) and ElevenLabs (TTS + music). ElevenLabs can "fetch voices" to list the voices available to your account. The request timeout defaults to 120 seconds (10–600).

---

## 2.13 Web search and web fetch

### Web search (web_search)

Configure search providers for the `web_search` tool in conversations.

**Where**: Settings → **Web Search** panel. 9 providers can be dragged to reorder (the one ranked first is the primary; the others that are enabled serve as fallbacks):

- **Free, no key needed**: DuckDuckGo (marked "limited reliability")
- **Self-hosted**: SearXNG (fill in the instance URL; includes one-click Docker deployment)
- **Key required**: Tavily (recommended globally), Bocha (recommended domestically), Brave, Perplexity, Google CSE, Grok, Kimi

| Advanced setting | What it does |
| --- | --- |
| Default result count | Number of results returned each time (1–10) |
| Timeout | 5–120 seconds |
| Cache TTL | 0–60 minutes |
| Default country / language / recency | Region, language, and freshness filters for the search |

There is no master switch; availability is determined by "whether there is an enabled provider." Provider keys can be configured by the AI through conversation, but are masked when read.

### Web fetch (web_fetch)

Configure fetch parameters for the `web_fetch` tool (pure parameters, no provider required).

**Where**: Settings → **Web Fetch** panel.

| Setting | Default | What it does |
| --- | --- | --- |
| Max characters | 50000 | The content limit injected into the model |
| Max response body | 2 MiB | The download size limit |
| Timeout | 30 seconds | 1–120 seconds |
| Max redirects | 5 | 0–20 |
| Cache TTL | 15 minutes | 0–1440 minutes |
| **SSRF protection** | **On** | Blocks access to intranet / metadata addresses to prevent server-side request forgery |

> **We recommend keeping SSRF protection on**. Turning it off allows intranet addresses and poses a security risk. All outbound requests follow a unified security policy; see [13 · Settings & Security](13-settings-and-security.md).

---

## Next steps

- Model configured, start chatting → [03 Chat & Sessions](03-chat-and-sessions.md)
- Let the AI remember your preferences → [04 Memory](04-memory.md)
- Learn where all the settings are → [13 Settings & Security](13-settings-and-security.md)
