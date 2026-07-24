// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { BUILTIN_DEBUG_PET_ASSET_ID } from "@/types/pet"

const mocks = vi.hoisted(() => ({
  loadPetAsset: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ loadPetAsset: mocks.loadPetAsset }),
}))

import { usePetAssetUrl } from "./usePetAssetUrl"

function Harness({ assetId }: { assetId: string | null }) {
  const asset = usePetAssetUrl(assetId)
  return (
    <span
      data-testid="asset"
      data-src={asset.src}
      data-loading={String(asset.loading)}
      data-failed={String(asset.failed)}
    />
  )
}

beforeEach(() => {
  mocks.loadPetAsset.mockReset()
})

afterEach(cleanup)

describe("usePetAssetUrl", () => {
  test("loads the debug atlas directly only in the development renderer", () => {
    render(<Harness assetId={BUILTIN_DEBUG_PET_ASSET_ID} />)

    expect(screen.getByTestId("asset")).toHaveAttribute(
      "data-src",
      expect.stringContaining("hope-debug.png"),
    )
    expect(screen.getByTestId("asset")).toHaveAttribute("data-loading", "false")
    expect(screen.getByTestId("asset")).toHaveAttribute("data-failed", "false")
    expect(mocks.loadPetAsset).not.toHaveBeenCalled()
  })
})
