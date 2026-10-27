// Client shell wrapper for the sign-in route. The page stays a server
// component that owns the OIDC-configured decision; this wrapper only renders
// the page inside the shared ExperienceShell and maps the safe error codes a
// provider redirect may append to the URL. It never echoes the raw parameter
// and it never changes an auth route, redirect, cookie, or session call.
"use client";

import { useSearchParams } from "next/navigation";
import type { ReactNode } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";

// Safe, human-readable messages keyed by the backend error code. The backend
// keeps emitting these codes; the UI only chooses a fixed message for each one.
const ERROR_MESSAGES: Record<string, string> = {
  unconfigured:
    "Sign-in is not available yet. An operator must supply an identity provider.",
  invalid_callback:
    "The sign-in response was incomplete or malformed. Please try again.",
  state_mismatch:
    "The sign-in response could not be verified against this browser session. Please sign in again.",
  missing_id_token:
    "The identity provider did not return an identity token. Please try again.",
  invalid_id_token:
    "The identity token could not be verified. Please sign in again.",
  token_exchange_failed:
    "The account service could not confirm the credential. Please try again.",
  provider_unavailable:
    "The identity provider is temporarily unavailable. Please try again later.",
  not_provisioned:
    "This account is not provisioned. Ask an administrator to enable it.",
  session_denied: "This account does not have access to the archive right now.",
  account_unavailable:
    "Your account could not be loaded. Please sign in again.",
};

const GENERIC_ERROR =
  "Sign-in could not be completed. Please try again or contact an administrator.";

export function signInErrorMessage(code: string | null): string | null {
  if (!code) {
    return null;
  }
  return ERROR_MESSAGES[code] ?? GENERIC_ERROR;
}

export function SignInShell({ children }: { children: ReactNode }) {
  const params = useSearchParams();
  const message = signInErrorMessage(params?.get("error") ?? null);

  return (
    <ExperienceShell>
      {() => (
        <div className="signin-layout">
          {message ? (
            <section
              className="state-panel state-panel-unavailable signin-error"
              data-testid="signin-error"
              role="alert"
            >
              <h2>Sign-in problem</h2>
              <p>{message}</p>
            </section>
          ) : null}
          {children}
        </div>
      )}
    </ExperienceShell>
  );
}
