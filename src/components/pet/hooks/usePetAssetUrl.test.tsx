// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react"
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
      data-fallback={String(asset.fallback)}
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
    expect(screen.getByTestId("asset")).toHaveAttribute("data-fallback", "false")
    expect(mocks.loadPetAsset).not.toHaveBeenCalled()
  })

  test("marks the bundled v2 asset as fallback while a custom asset is loading", () => {
    mocks.loadPetAsset.mockImplementation(() => new Promise(() => undefined))

    render(<Harness assetId="custom/v1" />)

    expect(screen.getByTestId("asset")).toHaveAttribute("data-loading", "true")
    expect(screen.getByTestId("asset")).toHaveAttribute("data-failed", "false")
    expect(screen.getByTestId("asset")).toHaveAttribute("data-fallback", "true")
  })

  test("keeps the bundled v2 fallback marker after a custom asset fails", async () => {
    mocks.loadPetAsset.mockRejectedValue(new Error("missing asset"))

    render(<Harness assetId="custom/v1" />)

    await waitFor(() => expect(screen.getByTestId("asset")).toHaveAttribute("data-failed", "true"))
    expect(screen.getByTestId("asset")).toHaveAttribute("data-loading", "false")
    expect(screen.getByTestId("asset")).toHaveAttribute("data-fallback", "true")
  })
})
