import type { ReactNode } from "react";

export const VERSION = "0.1.0";

export function PageFooter({ meta }: { meta?: ReactNode }) {
  return (
    <footer className="page-footer">
      {meta}
      <span className="footer-version">v{VERSION}</span>
    </footer>
  );
}
