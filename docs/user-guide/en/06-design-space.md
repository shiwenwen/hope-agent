# 06 · Design Space

The Design Space lets you collaborate with AI to **turn a single sentence or a reference image into deliverable design artifacts**—web pages, mobile prototypes, presentations, dashboards, posters, documents, emails, images, motion graphics, audio, and even genuinely interactive components. Generation previews in real time, with visual fine-tuning, version management, multi-format export, and a path all the way to a real code project.

**Entry point**: "Design Space" in the sidebar (right below "Knowledge Space").

**In this chapter**

- [6.1 Supported artifact types](#61-supported-artifact-types)
- [6.2 How to generate](#62-how-to-generate)
- [6.3 Preview, artifact library, and the workspace](#63-preview-artifact-library-and-the-workspace)
- [6.4 Editing artifacts through conversation](#64-editing-artifacts-through-conversation)
- [6.5 Visual fine-tuning and annotation](#65-visual-fine-tuning-and-annotation)
- [6.6 Version history](#66-version-history)
- [6.7 Export](#67-export)
- [6.8 Design systems and design tokens](#68-design-systems-and-design-tokens)
- [6.9 Handoff to a code project](#69-handoff-to-a-code-project)
- [6.10 Settings](#610-settings)

---

## 6.1 Supported artifact types

There are 11 forms in total. Most are self-contained HTML (rendered directly by the browser—fast to start, no blank screen); only the interactive component is compiled by the backend into a static artifact:

| Type | Description |
| --- | --- |
| Web page (web) | Landing page / desktop web prototype |
| Mobile prototype (mobile) | With a device frame and status bar |
| Presentation (deck) | Built-in pager, multiple pages in one file |
| Data dashboard (dashboard) | Grid-layout data board |
| Poster / social image (poster) | Size presets (square / portrait / A4, etc.) |
| Document / report (document) | A typeset reading container with a table of contents |
| Marketing email (email) | Layout compatible with email clients |
| Image (image) | Raster image produced by the image generation capability |
| Motion (motion) | Self-contained HTML animation |
| Audio (audio) | Text-to-speech / music / sound effects, with an embedded player |
| Interactive component (component) | A genuinely interactive React component / mini app, compiled by the backend into a static artifact |

> **Audio** and **image** artifacts both require a media generation provider: add one under Settings → Model Configuration → **Media Generation Models** (one key can carry several image / audio models), then pick the default model chains under Settings → Tool Settings → **Media Generation**. See [02 · Models & Providers](02-models-and-providers.md#212-ai-image-and-audio-generation). Even if an interactive component fails to compile, it degrades to a static error page rather than showing a blank screen.

---

## 6.2 How to generate

The Design Space home page is a large input box—one sentence gets you started, with no need to create a project first.

- **Generate from a sentence**: type a description and press `Cmd/Ctrl+Enter` to generate. Optionally pick an artifact type, a quick-select template, an inline design system, and a vision model.
- **Build from a reference image**: on the home page you can multi-select / paste / drag **up to 5** reference images, and you can generate from images alone without any text. The vision model **looks at the original image directly**, faithfully reproducing its colors, layout, and typographic feel. If the current model can't read images, it automatically switches to an available vision model and tells you; if no vision model is available at all, it stops and guides you to Settings.
- **Extract from a URL**: fetch a web page and its first-screen screenshot, and reverse-extract its brand design system.
- **Import from Figma**: read the published color / text / effect styles in Figma and distill them into a design system (**available only on desktop / the owner interface**; the Figma token is entered per use, never written to disk, and never sent to the model).
- **Extract from an existing code project**: derive a design system from a local code project's CSS / Tailwind config / design token files.
- **Brand kit**: generate multiple artifacts with reference images in one go.
- **Generate an image / audio artifact**: picking the "image" or "audio" type opens a dedicated generation dialog—images let you choose an aspect ratio and a resolution, audio lets you choose the type (speech / music / sound effects), a voice, and a duration. Only the options the configured models actually support are offered, and the dialog tells you which provider / model will be used. If no model is available for that capability, it shows an empty state that links straight to the media generation settings.

Generation is **truly streaming**, taking shape in the preview as it generates. Failures degrade to a clean placeholder page without interrupting the flow, and you can keep refining in the conversation.

---

## 6.3 Preview, artifact library, and the workspace

- **Single-artifact preview**: a stable preview window plus a zoom dropdown (pure CSS scaling). It **deliberately avoids an infinite canvas**—that was the source of the old version's lag.
- **Artifact library (thumbnail wall)**: a thumbnail wall of all artifacts, viewable across projects or within a project. Thumbnails are the artifact's real static preview (no scripts run, no lag).
- **Tab bar**: the top of the workspace lists only the artifacts you've "opened"; tabs can be closed (just removed from view, the file is not deleted), double-clicked to rename, and dragged to reorder. True **permanent deletion** lives in the right-click menu and the artifact library wall (irreversible).
- **Folder grouping**: artifacts can be sorted into folders, with drag-to-move and rename support (deleting a folder only returns its artifacts to the root—it does not delete them).
- **Device preview**: switch among `Auto / Desktop / Tablet / Phone` viewports; there is also a full-screen "presentation mode".

---

## 6.4 Editing artifacts through conversation

The left column of the workspace embeds an **AI chat panel**, the **primary entry point** for iterating on artifacts: tell the AI what to change in natural language, and the artifact lands a new version in place while the preview refreshes automatically.

- Each design project has its own conversation thread (hidden from the main session list and global search).
- Every turn, the panel automatically brings along "the artifact you currently have open" as context, so when you say "make **this** a dark version" or "give it a variant," the change lands on the artifact you're looking at.
- When empty, a getting-started card guides you; after the AI replies, next-step suggestions such as "more polished / dark version / make a variant / quality review" appear above the input box (clicking fills them in without auto-sending).

---

## 6.5 Visual fine-tuning and annotation

> These capabilities **apply only to the 8 forms that support element-level fine-tuning** (excluding image, audio, and interactive component).

**Visual fine-tuning**: click any element inside the artifact, and the inspector on the right shows 8 groups of controls—text, color, typography, spacing & radius, layout, size, stroke, and effects (shadow / opacity). Changes **preview with zero latency**, and once confirmed are automatically written back to the source and produce a new version; text can be double-clicked to edit in place. You can also change links and upload replacement images (automatically inlined to keep the artifact self-contained).

**Undo / redo**: `Cmd/Ctrl+Z` and `Cmd/Ctrl+Shift+Z`, up to 50 steps.

There are two kinds of **annotation**:

- **Element annotation pins**—drop an annotation pin on an element (it automatically re-anchors after design changes, so it isn't lost); the toolbar shows a badge with the count of unresolved annotations.
- **Frame / doodle annotations**—box out or circle a region on the preview and add a note.

Annotations can be "refined in one click" (let the AI apply the annotation in place) or "brought to the conversation" (inserted into the input box as a reference, to be augmented and then sent).

---

## 6.6 Version history

Every AI generation / refinement, visual fine-tune, design-system swap, and rollback produces a version snapshot.

**Version history** is two-column: the left column is the version list (with "AI-generated / manual fine-tune / rollback" tags, titles, timestamps, and search), and the right column is a live preview of the selected version. **Restoring** a historical version generates a **new version** based on it (the original history is untouched). The version cap defaults to 50; when exceeded, the oldest manual fine-tune versions are evicted first, preserving milestones like AI generations and rollbacks.

---

## 6.7 Export

Export prefers real, browser-native capture (highest fidelity), and when the conditions aren't available it automatically falls back to a client-side approach, so it **can always export**:

| Format | Use |
| --- | --- |
| HTML | Self-contained single file, ready to share / open directly |
| PNG | Full-fidelity screenshot (optional 1x / 2x / 3x sharpness) |
| PDF | Vector, with selectable and searchable text (presentations export one page per slide) |
| MP4 video | Recording of motion / interactive artifacts |
| PPTX | Presentation |
| ZIP | Packaged delivery |
| Markdown | Low-bandwidth delivery |
| Code delivery package | Contains HTML + source + six token-code formats + delivery notes, for the engineering side |

> Video and high-fidelity PDF/PNG require Chromium and ffmpeg—these are **not bundled into the installer**. Ones already installed on the system are used directly; missing ones are downloaded on demand, and any missing piece automatically degrades to the client-side approach without hanging. Desktop export pops the native "Save to…" dialog; web / server mode goes through a browser download, and **remote mode never writes to the server disk**.

---

## 6.8 Design systems and design tokens

A design system is a **reusable brand contract**—a `DESIGN.md` (covering 9 sections: brand, color, typography, spacing, layout, components, motion, tone, and taboos) plus a token table (`--ds-*` CSS variables). Applying it to an artifact is "reskinning," with consistency locked in by the tokens.

**Two kinds of built-in systems**:

- **6 original prototype languages**—Minimal Modern, Editorial Magazine, Tech Dark, Warm & Approachable, Professional Finance, and Bold & Vibrant, covering common temperaments.
- **A set of brand-style references**—covering mainstream brand styles. **These are all independent reinterpretations of each brand's public visual language, provided for design reference only**, and previewing / rendering automatically appends a one-line disclaimer (unofficial, no affiliation / authorization, trademarks belong to their respective owners). The original systems carry no disclaimer.

You can **reverse-extract** (one-click extraction of a design system from a screenshot / image / URL / code project / Figma), **fine-tune tokens visually** (adjust each variable by hand, with a color picker for colors), export tokens in one click to six developer formats—**CSS / SCSS / TypeScript / Swift (iOS) / Android XML / DTCG**—and import / export `DESIGN.md` as a cross-tool interchange format.

---

## 6.9 Handoff to a code project

The Design Space can push all the way to real code:

- **Bind a code repository**—a design project can be bound to a local directory or a Hope Agent project (**settable only on desktop / the owner interface**; the AI cannot authorize itself).
- **Implement to code**—the "Implement to code…" item in the artifact export menu hands the design off to an ordinary chat session that implements it in the bound repository, reusing the full [permission approval](07-tools-and-permissions.md) flow, the Diff panel, [Plan Mode](08-autonomous-tasks.md#85-plan-mode), and [isolated worktrees](07-tools-and-permissions.md#79-file-operations-git-and-isolated-worktrees)—every stroke of code goes through approval.
- **Code-change feedback**—after the implementation lands, if the code side then changes outside the conversation, the Design Space detects the "drift" and flags it on the artifact, prompting you to "view the code changes / bring to the design conversation / mark as synced."
- **Sync the design system to code**—bind the design system to a code directory and write the six token-format files into it in one click (re-sync after changing a token, and the code updates instantly).
- **Read-only sharing / deployment**—server mode can generate a read-only share link; optionally deploy to Cloudflare Pages or Vercel in one click (must be enabled manually, credentials configured only in the interface).

> **When requirements are unclear, the design AI asks first**: it pops a structured discovery questionnaire in the conversation, or—when a visual direction needs to be set—pops "visual style cards" (each option carries a color palette, a font sample, and a temperament description); the chosen one becomes that artifact's design system. When the requirements are written out clearly, it just gets started without fuss.

---

## 6.10 Settings

**Entry point**: Settings → Design Space (the whole category is medium risk, because it contains an "automatic quality review" that incurs model-call cost).

| Setting | Default | Effect |
| --- | --- | --- |
| Enable Design Space | On | Global switch |
| Auto-focus the preview after generation | On | Whether to automatically open the preview after generating an artifact |
| Default design system | None | Fallback design system for a new artifact when the project doesn't specify one |
| Auto-run a quality review before finalizing | **Off** | Runs a 5-dimension quality review on the artifact (brand fit / accessibility / visual hierarchy / usability / performance), which **incurs one model-call cost** |
| Anti-AI-slop self-check | On | A deterministic check with no model cost, catching near-empty / placeholder content and flagging it "needs review" |
| Versions retained per artifact | 50 | Version cap |
| Export sharpness / PDF quality | 2x / 92 | Rasterization scale and compression quality for export |

> **The Design Space is completely unavailable in Incognito sessions** and leaves zero trace (Incognito sessions themselves are mutually exclusive with projects / IM channels).

---

## Appendix: Canvas vs. Design Space

In the **main chat** you may encounter **Canvas**—a lightweight sandbox for casual, on-the-fly visuals in a conversation. The AI creates a visualization (HTML / Markdown / code / SVG / Mermaid / chart / slides) right in the chat and previews it in the right-side panel. It's suited to "sketch something quickly" and does no long-term management.

The **Design Space**, by contrast, is a systematic creative workspace with an artifact library, versions, export, design systems, and code binding. The two are independent and each play their part: use Canvas for casual visuals, and the Design Space for serious design work.

---

## Next steps

- Control the AI's permission to write code → [07 · Tools & Permissions](07-tools-and-permissions.md)
- Configure image / audio generation → [02 · Models & Providers](02-models-and-providers.md)
