// Sign-in route. The OIDC identity provider is operator-supplied; without it the
// page shows an explicit unsupported message rather than fabricating identity.

import { PageHeader } from "@/components/PageHeader";

export default function SignInPage() {
  return (
    <main className="app-shell">
      <PageHeader title="Sign in" subtitle="Sign in to use the genealogy workspace." />
      <section className="card" data-testid="signin-view">
        <p className="insufficient">
          Sign-in is not configured. An operator must supply an OIDC identity
          provider before accounts can sign in.
        </p>
      </section>
    </main>
  );
}
