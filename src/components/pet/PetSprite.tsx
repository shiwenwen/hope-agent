import { memo, useState } from "react"
import builtinPetUrl from "@/assets/pets/hope-default.webp"
import {
  usePetAnimator,
  type PetAction,
  type PetLookTarget,
} from "@/components/pet/hooks/usePetAnimator"

const CELL_WIDTH = 192
const CELL_HEIGHT = 208
const SHEET_WIDTH = CELL_WIDTH * 8

interface PetSpriteProps {
  src: string
  row: number
  frame: number
  rowCount: 1 | 9 | 11
  dimmed?: boolean
  onAssetError?: () => void
}

export const PetSprite = memo(function PetSprite({
  src,
  row,
  frame,
  rowCount,
  dimmed,
  onAssetError,
}: PetSpriteProps) {
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  const usingFallback = failedSrc === src
  const renderedRowCount = usingFallback ? 11 : rowCount
  return (
    <svg
      viewBox={`0 0 ${CELL_WIDTH} ${CELL_HEIGHT}`}
      aria-hidden="true"
      className={`h-[104px] w-[96px] transform-gpu overflow-hidden drop-shadow-lg ${dimmed || usingFallback ? "opacity-70" : "opacity-100"}`}
    >
      <image
        href={usingFallback ? builtinPetUrl : src}
        x={-frame * CELL_WIDTH}
        y={-row * CELL_HEIGHT}
        width={SHEET_WIDTH}
        height={CELL_HEIGHT * renderedRowCount}
        preserveAspectRatio="none"
        onError={() => {
          setFailedSrc(src)
          onAssetError?.()
        }}
      />
    </svg>
  )
})

interface AnimatedPetSpriteProps {
  src: string
  action: PetAction
  rowCount: 1 | 9 | 11
  lookTarget?: PetLookTarget
  dimmed?: boolean
  onActionComplete?: (action: PetAction) => void
}

/** Keeps frame ticks inside the sprite subtree so tray and bubble never render at animation FPS. */
export const AnimatedPetSprite = memo(function AnimatedPetSprite({
  src,
  action,
  rowCount,
  lookTarget = null,
  dimmed,
  onActionComplete,
}: AnimatedPetSpriteProps) {
  const animation = usePetAnimator(action, onActionComplete, rowCount === 11 ? lookTarget : null)
  return (
    <PetSprite
      src={src}
      row={animation.row}
      frame={animation.frame}
      rowCount={rowCount}
      dimmed={dimmed}
    />
  )
})
