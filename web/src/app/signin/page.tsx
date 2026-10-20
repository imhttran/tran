// Sign-in route. The OIDC identity provider is operator-supplied. When it is
// configured, the browser starts the server-side authorization-code + PKCE flow.
// Without it, the page says so rather than fabricating identity.

import { PageHeader } from "@/components/PageHeader";
import { oidcConfigured } from "@/lib/oidc";

export const dynamic = "force-dynamic";

export default function SignInPage() {
  const configured = oidcConfigured();
  return (
    <main className="app-shell">
      <PageHeader title="Sign in" subtitle="Sign in to use the genealogy workspace." />
      <section className="card" data-testid="signin-view">
        {configured ? (
          <>
            <p>
              Sign in through the configured identity provider. Your account is
              provisioned by an administrator before it can access family records.
            </p>
            <p>
              <a
                className="button"
                href="/api/auth/login"
                data-testid="signin-button"
              >
                Sign in
              </a>
            </p>
          </>
        ) : (
          <p className="insufficient">
            Sign-in is not configured. An operator must supply an OIDC identity
            provider before accounts can sign in.
          </p>
        )}
      </section>
    </main>
  );
}
