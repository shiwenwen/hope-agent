import { createContext, useContext } from "react"

export interface PortalScopeValue {
  active: boolean
  container: HTMLElement | null
}

export const PortalScopeContext = createContext<PortalScopeValue | null>(null)

/**
 * Persistent app surfaces stay mounted while hidden. Portaled UI must therefore
 * stay inside its owning surface and unmount while that surface is inactive;
 * otherwise a body-level modal can cover (and keep focus trapped over) another view.
 */
export function usePortalScope(): PortalScopeValue | null {
  return useContext(PortalScopeContext)
}
