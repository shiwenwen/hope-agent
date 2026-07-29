import { useMemo, type ReactNode } from "react"

import { PortalScopeContext, type PortalScopeValue } from "./portal-scope-context"

export function PortalScopeProvider({
  active,
  container,
  children,
}: PortalScopeValue & { children: ReactNode }) {
  const value = useMemo(() => ({ active, container }), [active, container])
  return <PortalScopeContext.Provider value={value}>{children}</PortalScopeContext.Provider>
}
