"use client";

import { useEffect, useState } from "react";
import type { FormEvent } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { FormStatus } from "@/components/FormStatus";

// Landing page for a shared invitation link (/activate?token=...). The token is
// removed from the address bar at once, then handed to /api/auth/activate as an
// HttpOnly cookie; the sign-in callback redeems it and returns here with
// ?result=. Nothing on this page grants access: an administrator confirms each
// claim before the account is linked to a person.
const RESULTS: Record<string, { title: string; text: string; ok?: boolean }> = {
  awaiting: {
    ok: true,
    title: "Thank you",
    text: "Your invitation is recorded. An administrator will confirm that this login belongs to you; once they do, you can see the family archive. You can close this page.",
  },
  expired: {
    title: "This invitation has expired",
    text: "Ask the person who sent it for a new link.",
  },
  used: {
    title: "This invitation was already used",
    text: "Each link works once. If you did not use it, ask the person who sent it for a new link.",
  },
  invalid: {
    title: "This invitation link is not valid",
    text: "Check that you opened the whole link, or ask the person who sent it for a new one.",
  },
  linked: {
    title: "Already linked",
    text: "This login, or the family member this invitation is for, is already linked. If that is unexpected, contact an administrator.",
  },
  denied: {
    title: "This login cannot accept invitations",
    text: "Contact an administrator.",
  },
  error: {
    title: "Something went wrong",
    text: "The invitation could not be recorded. Try the same link again later.",
  },
};

export default function ActivatePage() {
  const [token, setToken] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    // Only set what is present: the token is stripped from the address below,
    // so a re-run of this effect must not clear the token already read.
    const found = params.get("token");
    if (found) {
      setToken(found);
      // Keep the single-use token out of history, bookmarks, and screenshots.
      window.history.replaceState(null, "", "/activate");
    }
    const outcome = params.get("result");
    if (outcome) setResult(outcome);
    setReady(true);
  }, []);

  async function onContinue(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!token || busy || name.trim() === "") return;
    setBusy(true);
    setError(null);
    try {
      const response = await fetch("/api/auth/activate", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ token, name: name.trim() }),
      });
      if (!response.ok) {
        const body = (await response.json().catch(() => null)) as { error?: string } | null;
        setError(
          body?.error === "invalid_name"
            ? "Enter your name (up to 100 characters)."
            : "This invitation link is not valid. Ask the person who sent it for a new one.",
        );
        setBusy(false);
        return;
      }
      window.location.href = "/api/auth/login";
    } catch {
      setError("The invitation could not be saved. Check your connection and try again.");
      setBusy(false);
    }
  }

  const outcome = result ? RESULTS[result] ?? RESULTS.error : null;
  return (
    <ExperienceShell>
      {() => (
      <>
      <title>Accept invitation</title>
      <section className="card signin-panel" data-testid="activate-view">
        <p className="signin-eyebrow">Family archive</p>
        {!ready ? null : outcome ? (
          <div data-testid={`activate-result-${result}`}>
            <h1 className="signin-title">{outcome.title}</h1>
            <p className="signin-lead">{outcome.text}</p>
          </div>
        ) : token ? (
          <>
            <h1 className="signin-title">You are invited</h1>
            <p className="signin-lead">
              You have been invited to the family archive. Tell us your name,
              then sign in. If you do not have a login yet, choose{" "}
              <strong>Register</strong> on the sign-in page to create one.
            </p>
            <form onSubmit={onContinue} data-testid="activate-form">
              <label htmlFor="activate-name">Your name</label>
              <input
                id="activate-name"
                type="text"
                autoComplete="name"
                maxLength={100}
                value={name}
                onChange={(event) => setName(event.target.value)}
                data-testid="activate-name"
              />
              <p className="hint">
                An administrator uses this to confirm the invitation reached the
                right person.
              </p>
              <button type="submit" disabled={busy || name.trim() === ""} data-testid="activate-continue">
                {busy ? "Continuing…" : "Continue to sign in"}
              </button>
            </form>
            <FormStatus status={error ? { kind: "error", text: error } : null} />
          </>
        ) : (
          <div data-testid="activate-missing">
            <h1 className="signin-title">Invitation</h1>
            <p className="signin-lead">
              Open this page from the invitation link you were sent.
            </p>
          </div>
        )}
      </section>
      </>
      )}
    </ExperienceShell>
  );
}
