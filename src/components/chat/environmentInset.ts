export interface EnvironmentInsetInput {
  /** Width of the conversation column. */
  available: number
  /** Width the floating card needs, measured from the column's right edge. */
  lane: number
  /** Width the centred transcript/composer column caps at. */
  contentMaxWidth: number
  /** Below this the lane is dropped and the card is allowed to overlap. */
  minContentWidth: number
}

/**
 * Right-hand lane to keep clear for the environment card, or 0 to leave the
 * layout alone. The column shifts left first and only shrinks once shifting
 * runs out of room, because the caller applies this as padding around a
 * centred, max-width column.
 */
export function environmentInsetWidth({
  available,
  lane,
  contentMaxWidth,
  minContentWidth,
}: EnvironmentInsetInput): number {
  if (available <= 0 || lane <= 0) return 0
  const centeredRight = (available + Math.min(contentMaxWidth, available)) / 2
  const laneLeft = available - lane
  if (centeredRight <= laneLeft) return 0
  if (laneLeft < minContentWidth) return 0
  return lane
}
