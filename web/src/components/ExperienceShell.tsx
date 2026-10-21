"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";

import { AccountMenu } from "@/components/AccountMenu";
import { canUpload } from "@/lib/experience-contract";
import type { Role, SessionResult } from "@/lib/experience-contract";

// Client shell for authenticated pages. It resolves the session from the
// same-origin session endpoint (the server never sends a token, subject, email,
// or credential to the browser) and renders the shared navigation and account
// menu around the page. The role is passed to the page so it can hide controls,
// but it is never an authorization decision: Rust enforces every request.
type Props = {
  children: (role: Role | null, session: SessionResult | null) => ReactNode;
};

// Only implemented routes. "Add documents" is the existing direct-upload page.
const NAV_ITEMS = [
  { href: "/profile", label: "My profile", icon: "☺" },
  { href: "/genealogy", label: "Family tree", icon: "♧" },
  { href: "/ask", label: "Ask the archive", icon: "?" },
  { href: "/head-of-family", label: "Head of Family", icon: "◈" },
  { href: "/branch-review", label: "Branch review", icon: "⚖" },
];
const UPLOAD_ITEM = { href: "/", label: "Add documents", icon: "＋" };
const ADMIN_ITEMS = [
  { href: "/admin/reviews", label: "Reviews", icon: "✓" },
  { href: "/admin/family-members", label: "Bulk onboard", icon: "✦" },
];

async function readSession(): Promise<SessionResult> {
  try {
    const response = await fetch("/api/auth/session", { cache: "no-store" });
    if (response.status === 401) {
      return { kind: "expired", signInUrl: "/signin" };
    }
    if (response.status === 403) {
      return {
        kind: "denied",
        reason: "Your role does not permit this request.",
      };
    }
    if (!response.ok) {
      return {
        kind: "denied",
        reason: `The session service returned HTTP ${response.status}.`,
      };
    }
    const body = (await response.json()) as {
      state?: string;
      role?: Role;
      display_name?: string;
    };
    if (body.state === "signed_in") {
      const role = body.role ?? null;
      return {
        kind: "allowed",
        data: {
          state: "signed_in",
          role,
          display_name:
            body.display_name ??
            (role ? `Signed-in ${role}` : "Signed-in member"),
        },
      };
    }
    return { kind: "allowed", data: { state: "anonymous" } };
  } catch {
    return { kind: "denied", reason: "The session service is unreachable." };
  }
}

function roleOf(session: SessionResult | null): Role | null {
  if (session?.kind === "allowed" && session.data.state === "signed_in") {
    return session.data.role;
  }
  return null;
}

export function ExperienceShell({ children }: Props) {
  const pathname = usePathname();
  const [session, setSession] = useState<SessionResult | null>(null);
  const [signingOut, setSigningOut] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    readSession().then((result) => {
      if (!cancelled) setSession(result);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  function signIn() {
    window.location.href = "/signin";
  }

  async function signOut() {
    if (signingOut) return;
    setSigningOut(true);
    try {
      await fetch("/api/auth/signout", { method: "POST" });
    } catch {
      // The session is cleared server-side when possible; the reload below still
      // makes the client re-read the authoritative session.
    } finally {
      window.location.reload();
    }
  }

  const role = roleOf(session);
  const items = [
    ...NAV_ITEMS,
    ...(role === "admin" ? ADMIN_ITEMS : []),
    ...(canUpload(role) ? [UPLOAD_ITEM] : []),
  ];
  return (
    <div className="experience">
      <aside className="site-nav" data-testid="site-nav">
        <div className="site-nav-bar">
          <Link href="/genealogy" className="brand">
            TRẦN FAMILY<small>Family archive</small>
          </Link>
          <button
            type="button"
            className="site-nav-toggle"
            aria-expanded={menuOpen}
            aria-controls="site-nav-links"
            onClick={() => setMenuOpen((open) => !open)}
            data-testid="nav-toggle"
          >
            {menuOpen ? "Close" : "Menu"}
          </button>
        </div>
        <nav
          id="site-nav-links"
          aria-label="Main navigation"
          className={menuOpen ? "open" : undefined}
        >
          {items.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              aria-current={pathname === item.href ? "page" : undefined}
              onClick={() => setMenuOpen(false)}
            >
              <span aria-hidden="true">{item.icon}</span> {item.label}
            </Link>
          ))}
        </nav>
      </aside>
      <main className="app-shell">
        {session ? (
          <AccountMenu
            session={session}
            signingOut={signingOut}
            onSignIn={signIn}
            onSignOut={signOut}
          />
        ) : null}
        {children(role, session)}
      </main>
    </div>
  );
}
