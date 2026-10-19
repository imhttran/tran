import type { ReactNode } from "react";

// Minimal shell. The reference AppShell carries domain nav (inventory-specific);
// this prototype has no nav — an intentional difference recorded in the plan.
export function AppShell({ children }: { children: ReactNode }) {
  return <main className="app-shell">{children}</main>;
}
