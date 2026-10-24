"use client";

import type { SessionResult } from "@/lib/experience-contract";

// The account menu renders only from the session result. It shows a sign-in
// action when the session is anonymous or expired and a sign-out action when
// signed in. It never renders an account identifier, email, subject, token, or
// identity claim; the server chooses the display label and role.
type Props = {
  session: SessionResult;
  signingOut: boolean;
  onSignIn: () => void;
  onSignOut: () => void;
};

export function AccountMenu({ session, signingOut, onSignIn, onSignOut }: Props) {
  if (session.kind === "expired") {
    return (
      <nav className="account-menu" data-testid="account-menu" data-state="expired">
        <p className="hint" data-testid="session-expired">
          Session expired. Sign in again.
        </p>
        <button type="button" onClick={onSignIn} data-testid="sign-in-button">
          Sign in again
        </button>
      </nav>
    );
  }

  if (session.kind === "denied" || session.kind === "no_access") {
    return (
      <nav className="account-menu" data-testid="account-menu" data-state="signed-out">
        <button type="button" onClick={onSignIn} data-testid="sign-in-button">
          Sign in
        </button>
      </nav>
    );
  }

  const view = session.data;
  if (view.state === "anonymous") {
    return (
      <nav className="account-menu" data-testid="account-menu" data-state="signed-out">
        <button type="button" onClick={onSignIn} data-testid="sign-in-button">
          Sign in
        </button>
      </nav>
    );
  }

  return (
    <nav className="account-menu" data-testid="account-menu" data-state="signed-in">
      <span className="meta" data-testid="account-label">
        {view.display_name}
      </span>
      <span className="badge" data-testid="account-role">
        {view.role ?? "member"}
      </span>
      <button
        type="button"
        className="button-secondary"
        onClick={onSignOut}
        disabled={signingOut}
        data-testid="sign-out-button"
      >
        {signingOut ? "Signing out…" : "Sign out"}
      </button>
    </nav>
  );
}
