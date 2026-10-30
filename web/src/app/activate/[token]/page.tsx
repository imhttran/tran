"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  acceptInvitation,
  previewInvitation,
  type InvitationActivationResponse,
  type InvitationPreviewResponse,
} from "@/lib/api";

// PIR-006: the invitation welcome and activation screen.
//
// The invitation link is the credential. This page validates it, shows the
// intended person so the reader can confirm they are the right person, and
// offers two paths: continue through the existing identity provider, or leave
// (the "This is not me" path). Activation is explicit: it links the signed-in
// account to the invited person and consumes the one-time link. It never creates
// an account, never replaces an existing link, and never grants a role.
//
// The token is used only in the request path. It is never rendered, logged, or
// stored.

type Problem = { title: string; body: string };

type View =
  | { kind: "loading" }
  | { kind: "problem"; problem: Problem }
  | { kind: "ready"; preview: InvitationPreviewResponse };

function activationPath(token: string): string {
  return `/activate/${encodeURIComponent(token)}`;
}

// Maps a preview failure to a fixed message. The raw error code is never shown.
function previewProblem(err: unknown): Problem {
  if (err instanceof ApiRequestError) {
    if (err.status === 404) {
      return {
        title: "This invitation link is not valid",
        body: "The link may be incomplete. Ask the family administrator to send it again.",
      };
    }
    if (err.status === 410) {
      return {
        title: "This invitation is no longer available",
        body: "Ask the family administrator for a new invitation.",
      };
    }
  }
  return {
    title: "The invitation could not be checked",
    body: "Try again in a moment. If it keeps failing, ask the family administrator to resend the invitation.",
  };
}

// Maps an activation failure to a fixed message and next step.
function activationProblem(err: unknown): Problem {
  if (err instanceof ApiRequestError) {
    switch (err.status) {
      case 401:
        return {
          title: "Sign in to accept",
          body: "Sign in first, then open this invitation link again.",
        };
      case 403:
        return {
          title: "Pending review",
          body: "Your account is not ready to accept an invitation yet. An administrator must enable it. Nothing has been linked.",
        };
      case 404:
        return {
          title: "This invitation link is not valid",
          body: "Ask the family administrator to send a new invitation.",
        };
      case 409:
        if (err.body?.error === "wrong_person") {
          return {
            title: "This invitation is for a different person",
            body: "Your signed-in account is already linked to someone else, so this link cannot be used for that account.",
          };
        }
        if (err.body?.error === "person_already_linked") {
          return {
            title: "This person already has an account",
            body: "Someone else accepted this invitation first. Ask the family administrator if you think that is wrong.",
          };
        }
        if (err.body?.error === "invitation_accepted") {
          return {
            title: "This invitation was already used",
            body: "Sign in with the account that accepted it, or ask for a new invitation.",
          };
        }
        break;
      case 410:
        return {
          title: "This invitation is no longer available",
          body: "It expired or was cancelled. Ask the family administrator for a new invitation.",
        };
      default:
        break;
    }
  }
  return {
    title: "The invitation could not be accepted",
    body: "Nothing was changed. Try again, or ask the family administrator for help.",
  };
}

function StatePanel({
  problem,
  children,
}: {
  problem: Problem;
  children?: ReactNode;
}) {
  return (
    <section
      className="state-panel"
      data-testid="activation-problem"
      role="alert"
    >
      <h1>{problem.title}</h1>
      <p>{problem.body}</p>
      <div className="state-panel-actions">{children}</div>
    </section>
  );
}

function ActivationView({ token }: { token: string }) {
  const [view, setView] = useState<View>({ kind: "loading" });
  const [signedIn, setSignedIn] = useState<boolean | null>(null);
  const [accepted, setAccepted] = useState<InvitationActivationResponse | null>(
    null,
  );
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busy, setBusy] = useState(false);
  const sequence = useRef(0);

  useEffect(() => {
    const current = ++sequence.current;
    async function load() {
      if (!token) {
        setView({
          kind: "problem",
          problem: {
            title: "This invitation link is not valid",
            body: "Ask the family administrator to send a new invitation.",
          },
        });
        return;
      }
      try {
        const preview = await previewInvitation(token);
        if (sequence.current !== current) return;
        setView({ kind: "ready", preview });
      } catch (err) {
        if (sequence.current !== current) return;
        setView({ kind: "problem", problem: previewProblem(err) });
      }
      try {
        // The session view carries no credential. An anonymous visitor simply
        // gets the identity-provider path below.
        const response = await fetch("/api/auth/session", {
          cache: "no-store",
        });
        const body = (await response.json()) as { state?: string };
        if (sequence.current !== current) return;
        setSignedIn(body.state === "signed_in");
      } catch {
        if (sequence.current !== current) return;
        setSignedIn(false);
      }
    }
    void load();
    return () => {
      sequence.current += 1;
    };
  }, [token]);

  const accept = useCallback(async () => {
    setBusy(true);
    setProblem(null);
    try {
      const result = await acceptInvitation(token);
      setAccepted(result);
    } catch (err) {
      setProblem(activationProblem(err));
    } finally {
      setBusy(false);
    }
  }, [token]);

  const leave = useCallback(async () => {
    try {
      await fetch("/api/auth/signout", { method: "POST" });
    } catch {
      // The navigation below still leaves this invitation link.
    } finally {
      window.location.href = "/signin";
    }
  }, []);

  if (accepted) {
    return (
      <>
        <PageTitle title="Invitation accepted" />
        <section
          className="state-panel"
          data-testid="activation-accepted"
          role="status"
        >
          <h1>You&apos;re all set</h1>
          <p>
            Your account is now linked to {accepted.display_name} with Member
            access. Nothing else was changed.
          </p>
          <div className="state-panel-actions">
            <Link
              href={`/family/${encodeURIComponent(accepted.person_key)}`}
              className="button"
              data-testid="activation-open-profile"
            >
              Open the family profile
            </Link>
          </div>
        </section>
      </>
    );
  }

  if (problem) {
    return (
      <>
        <PageTitle title="Invitation" />
        <StatePanel problem={problem}>
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  if (view.kind === "loading") {
    return (
      <>
        <PageTitle title="Invitation" />
        <section
          className="state-panel state-panel-loading"
          role="status"
          aria-busy="true"
          data-testid="activation-loading"
        >
          <h1>Checking your invitation…</h1>
        </section>
      </>
    );
  }

  if (view.kind === "problem") {
    return (
      <>
        <PageTitle title="Invitation" />
        <StatePanel problem={view.problem}>
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  const { preview } = view;

  if (preview.state === "expired") {
    return (
      <>
        <PageTitle title="Invitation expired" />
        <StatePanel
          problem={{
            title: "This invitation has expired",
            body: "Invitation links are valid for a limited time. Ask the family administrator for a new invitation.",
          }}
        >
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  if (preview.state === "revoked") {
    return (
      <>
        <PageTitle title="Invitation cancelled" />
        <StatePanel
          problem={{
            title: "This invitation was cancelled",
            body: "A family administrator revoked this link, so it can no longer be used.",
          }}
        >
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  if (preview.state === "accepted") {
    return (
      <>
        <PageTitle title="Invitation already used" />
        <StatePanel
          problem={{
            title: "This invitation was already used",
            body: "If that was you, sign in to open the family archive. Otherwise ask the family administrator for a new invitation.",
          }}
        >
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  if (preview.person_linked) {
    return (
      <>
        <PageTitle title="Person already has an account" />
        <StatePanel
          problem={{
            title: "This person already has an account",
            body: "A family account is already linked to this person. Sign in to use it, or ask the family administrator if this looks wrong.",
          }}
        >
          <Link href="/signin" className="button-secondary">
            Go to sign in
          </Link>
        </StatePanel>
      </>
    );
  }

  const signInHref = `/api/auth/login?returnTo=${encodeURIComponent(
    activationPath(token),
  )}`;

  return (
    <>
      <PageTitle title="Welcome to the family archive" />
      <section
        className="card signin-panel"
        data-testid="activation-welcome"
        aria-labelledby="activation-heading"
      >
        <p className="signin-eyebrow">Family archive invitation</p>
        <h1 id="activation-heading" className="signin-title">
          You&apos;re invited
        </h1>
        <div className="signin-body">
          <div className="signin-main">
            <p className="signin-lead" data-testid="activation-person">
              This invitation is for <strong>{preview.display_name}</strong>.
            </p>
            <p>
              Accepting links your own account to this person with Member
              access. You can read the family archive and fill in your own
              profile. It does not make you an administrator.
            </p>

            {signedIn ? (
              <p className="signin-action">
                <button
                  type="button"
                  className="button signin-cta"
                  onClick={() => void accept()}
                  disabled={busy}
                  data-testid="activation-accept"
                >
                  Accept invitation
                </button>
              </p>
            ) : (
              <p className="signin-action">
                <a
                  className="button signin-cta"
                  href={signInHref}
                  data-testid="activation-signin"
                >
                  Continue with your identity provider
                </a>
              </p>
            )}

            <p className="hint" data-testid="activation-not-me">
              {signedIn ? (
                <button
                  type="button"
                  className="button-secondary"
                  onClick={() => void leave()}
                  disabled={busy}
                >
                  This is not me
                </button>
              ) : (
                <>
                  This is not you?{" "}
                  <Link href="/signin">Sign in with your own account</Link>.
                </>
              )}
            </p>
          </div>
          <aside className="signin-side">
            <h2>What happens next</h2>
            <ul>
              <li>You sign in with the identity provider your family uses.</li>
              <li>Your account is linked to this person only.</li>
              <li>The invitation link then stops working.</li>
            </ul>
          </aside>
        </div>
      </section>
      {busy ? (
        <p className="hint" role="status" data-testid="activation-working">
          Linking your account…
        </p>
      ) : null}
    </>
  );
}

// Invitation links are opened from a mail client, so the token is a path
// parameter. The shell renders the anonymous navigation; the view decides.
export default function ActivatePage() {
  const params = useParams<{ token: string }>();
  const raw = params?.token;
  const token = Array.isArray(raw) ? raw[0] : (raw ?? "");
  return (
    <ExperienceShell>
      {() => <ActivationView key={token} token={token} />}
    </ExperienceShell>
  );
}
