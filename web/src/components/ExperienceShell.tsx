"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";

import { AccountMenu } from "@/components/AccountMenu";
import { canUpload } from "@/lib/experience-contract";
import type { Role, SessionResult } from "@/lib/experience-contract";
import {
  markSessionExpired,
  onSessionExpired,
  sessionIsExpired,
} from "@/lib/session-expiry";

// Client shell for authenticated pages. It resolves the session from the
// same-origin session endpoint (the server never sends a token, subject, email,
// or credential to the browser) and renders the shared navigation and account
// menu around the page. The role is passed to the page so it can hide controls,
// but it is never an authorization decision: Rust enforces every request.
type Props = {
  children: (role: Role | null, session: SessionResult | null) => ReactNode;
};

// One line-icon set, drawn with currentColor, so states come from CSS color and
// opacity rather than separate assets. 1.5px stroke matches regular text weight.
type NavIconName =
  | "profile"
  | "tree"
  | "ask"
  | "head"
  | "review"
  | "documents"
  | "upload"
  | "admin-reviews"
  | "bulk-onboard";

function NavIcon({ name }: { name: NavIconName }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {name === "profile" && (
        <>
          <circle cx="12" cy="8" r="3.5" />
          <path d="M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6" />
        </>
      )}
      {name === "tree" && (
        <>
          <circle cx="12" cy="5" r="2.5" />
          <circle cx="6" cy="19" r="2.5" />
          <circle cx="18" cy="19" r="2.5" />
          <path d="M12 7.5v4M12 11.5 6 16.5M12 11.5l6 5" />
        </>
      )}
      {name === "ask" && (
        <>
          <circle cx="11" cy="11" r="6" />
          <path d="m16 16 4 4" />
        </>
      )}
      {name === "head" && (
        <path d="m12 3 2.4 5 5.6.8-4 4 1 5.6-5-2.7-5 2.7 1-5.6-4-4 5.6-.8z" />
      )}
      {name === "review" && (
        <>
          <path d="M12 4v16M7 20h10M5 8h14" />
          <path d="M5 8 3 13a3 3 0 0 0 6 0L7 8M19 8l-2 5a3 3 0 0 0 6 0l-2-5" />
        </>
      )}
      {name === "documents" && (
        <>
          <path d="M7 3h7l4 4v14H7z" />
          <path d="M14 3v4h4" />
        </>
      )}
      {name === "upload" && <path d="M12 5v14M5 12h14" />}
      {name === "admin-reviews" && <path d="m5 12 4 4 10-10" />}
      {name === "bulk-onboard" && (
        <>
          <circle cx="9" cy="8" r="3" />
          <path d="M3 20c0-3 2.7-5 6-5s6 2 6 5" />
          <path d="M16 5.5a3 3 0 0 1 0 5.5M17 15c2.3.4 4 2.3 4 5" />
        </>
      )}
    </svg>
  );
}

// Only implemented routes. "Add documents" is the existing direct-upload page.
type NavItem = { href: string; label: string; icon: NavIconName };
const NAV_ITEMS: NavItem[] = [
  { href: "/profile", label: "My profile", icon: "profile" },
  { href: "/genealogy", label: "Family tree", icon: "tree" },
  { href: "/ask", label: "Ask the archive", icon: "ask" },
  { href: "/head-of-family", label: "Head of Family", icon: "head" },
  { href: "/branch-review", label: "Branch review", icon: "review" },
  { href: "/documents", label: "Documents", icon: "documents" },
];
const UPLOAD_ITEM: NavItem = {
  href: "/",
  label: "Add documents",
  icon: "upload",
};
const ADMIN_ITEMS: NavItem[] = [
  { href: "/admin/reviews", label: "Reviews", icon: "admin-reviews" },
  {
    href: "/admin/document-extraction",
    label: "Extractions",
    icon: "documents",
  },
  {
    href: "/admin/source-of-truth",
    label: "Source of truth",
    icon: "documents",
  },
  {
    href: "/admin/family-members",
    label: "Bulk onboard",
    icon: "bulk-onboard",
  },
];

async function readSession(): Promise<SessionResult> {
  try {
    const response = await fetch("/api/auth/session", { cache: "no-store" });
    if (response.status === 401) {
      markSessionExpired();
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
    // A concurrent protected request may already have observed a 401 and
    // cleared the cookie. Do not overwrite that expired state with anonymous.
    if (sessionIsExpired()) {
      return { kind: "expired", signInUrl: "/signin" };
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
    // A protected call may report 401 before or after the initial session read.
    // This subscription is the single place that turns that into the expired
    // shell state, so concurrent failures do not reload or redirect.
    const unsubscribe = onSessionExpired(() => {
      if (!cancelled) {
        setSession({ kind: "expired", signInUrl: "/signin" });
      }
    });
    readSession().then((result) => {
      if (!cancelled) setSession(result);
    });
    return () => {
      cancelled = true;
      unsubscribe();
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
              <NavIcon name={item.icon} />
              <span>{item.label}</span>
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
