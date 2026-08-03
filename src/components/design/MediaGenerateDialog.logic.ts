import type {
  ImageEditCaps,
  ImageModelCaps,
  MediaAudioKind,
  MediaCandidateOverview,
} from "@/components/settings/media-gen/types"

export function inferAudioKindFromPrompt(prompt: string): MediaAudioKind | null {
  const lead = prompt.trimStart().toLowerCase()
  return lead.startsWith("[music]") ? "music" : lead.startsWith("[sfx]") ? "sfx" : null
}

export function resolveImageRequestCapability(
  candidates: MediaCandidateOverview[],
  referenceImageCount: number,
): {
  candidate: MediaCandidateOverview | null
  caps: ImageModelCaps | ImageEditCaps | null
} {
  if (referenceImageCount <= 0) {
    const candidate = candidates[0] ?? null
    return { candidate, caps: candidate?.image ?? null }
  }

  const candidate =
    candidates.find((item) => {
      const edit = item.image?.edit
      return edit != null && edit.maxInputImages >= referenceImageCount
    }) ?? null
  return { candidate, caps: candidate?.image?.edit ?? null }
}
