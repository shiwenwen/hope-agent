export type UnsupportedModelBehavior = "disable" | "hide"

/** A model must support every requested input type to remain selectable. */
export function modelSupportsInputTypes(
  inputTypes: string[] | undefined,
  requiredInputTypes: string[] | undefined,
): boolean {
  if (!requiredInputTypes?.length) return true
  if (!inputTypes?.length) return false
  return requiredInputTypes.every((inputType) => inputTypes.includes(inputType))
}
