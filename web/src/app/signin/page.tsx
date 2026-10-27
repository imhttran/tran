// Sign-in route. The OIDC identity provider is operator-supplied. When it is
// configured, the browser starts the server-side authorization-code + PKCE flow.
// Without it, the page says so rather than fabricating identity.

import { SignInShell } from "./SignInShell";
import { oidcConfigured } from "@/lib/oidc";

export const dynamic = "force-dynamic";

export default function SignInPage() {
  const configured = oidcConfigured();
  return (
    <SignInShell>
      <section
        className={
          configured
            ? "card signin-panel"
            : "state-panel state-panel-unavailable signin-panel"
        }
        data-testid="signin-view"
      >
        <p className="signin-eyebrow">Family archive</p>
        <h1 className="signin-title">Sign in</h1>
        {configured ? (
          <div className="signin-body">
            <div className="signin-main">
              <p className="signin-lead">
                Sign in to use the genealogy workspace. Your account is
                provisioned by an administrator before it can access family
                records.
              </p>
              <p className="signin-action">
                <a
                  className="button signin-cta"
                  href="/api/auth/login"
                  data-testid="signin-button"
                >
                  Sign in
                </a>
              </p>
            </div>
            <aside className="signin-side">
              <h2>What you need</h2>
              <ul>
                <li>A provisioned account from an administrator.</li>
                <li>Access to the identity provider your family uses.</li>
              </ul>
            </aside>
          </div>
        ) : (
          <p className="signin-lead">
            Sign-in is not configured. An operator must supply an OIDC identity
            provider before accounts can sign in.
          </p>
        )}
      </section>
    </SignInShell>
  );
}
