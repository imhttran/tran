"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { AccountMenu } from "@/components/AccountMenu";
import { decisionsNeedingAction } from "@/lib/api";
import type { ProfileResponse } from "@/lib/api";
import { canUpload } from "@/lib/experience-contract";
import type { Role, SessionResult } from "@/lib/experience-contract";
import {
  markSessionExpired,
  onSessionExpired,
  sessionIsExpired,
} from "@/lib/session-expiry";

// Client shell for authenticated pages. It resolves the session from the
// same-origin session endpoint (the server never sends a token, subject, email,
// or credential to the browser) and renders the shared top header, navigation,
// and account menu around the page. The role is passed to the page so it can
// hide controls, but it is never an authorization decision: Rust enforces every
// request.
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
  | "gallery"
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
      {name === "gallery" && (
        <>
          <rect x="4" y="5" width="16" height="14" rx="2" />
          <circle cx="9" cy="10" r="1.5" />
          <path d="m4 17 5-5 4 4 3-3 4 4" />
        </>
      )}
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

// Three primary destinations stay direct links in the desktop header.
const PRIMARY_ITEMS: NavItem[] = [
  { href: "/genealogy", label: "Family tree", icon: "tree" },
  { href: "/ask", label: "Ask the archive", icon: "ask" },
  { href: "/documents", label: "Documents", icon: "documents" },
];

// Every remaining permitted destination goes into More on desktop and into the
// Menu on mobile, in this order. Gating is presentation only.
const SECONDARY_ITEMS: NavItem[] = [
  { href: "/profile", label: "My profile", icon: "profile" },
  { href: "/gallery", label: "Photograph gallery", icon: "gallery" },
  { href: "/head-of-family", label: "Head of Family", icon: "head" },
  { href: "/branch-review", label: "Branch review", icon: "review" },
];
const UPLOAD_ITEM: NavItem = {
  href: "/",
  label: "Add documents",
  icon: "upload",
};
const ADMIN_ITEMS: NavItem[] = [
  {
    href: "/admin/narrative-review",
    label: "Personal history review",
    icon: "admin-reviews",
  },
  { href: "/admin/reviews", label: "Reviews", icon: "admin-reviews" },
  {
    href: "/admin/photographs",
    label: "Photographs",
    icon: "gallery",
  },
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

function isCurrent(pathname: string | null, href: string): boolean {
  if (!pathname) return false;
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(`${href}/`);
}

function matchesAny(pathname: string | null, items: NavItem[]): boolean {
  return items.some((item) => isCurrent(pathname, item.href));
}

export function ExperienceShell({ children }: Props) {
  const pathname = usePathname();
  const [session, setSession] = useState<SessionResult | null>(null);
  const [signingOut, setSigningOut] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);
  const moreButtonRef = useRef<HTMLButtonElement | null>(null);
  const mobileRef = useRef<HTMLDivElement | null>(null);
  const mobileButtonRef = useRef<HTMLButtonElement | null>(null);

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

  // Review decisions waiting on this member. Re-read on navigation so the
  // count clears after they revise on My profile. This is a hint only: it uses
  // a plain fetch so its own 401 or error never changes the session state, and
  // any failure just hides the badge.
  const signedIn = roleOf(session) !== null;
  const [actionCount, setActionCount] = useState(0);
  useEffect(() => {
    if (!signedIn) {
      setActionCount(0);
      return;
    }
    let cancelled = false;
    fetch("/api/profile", { cache: "no-store" })
      .then(async (raw) => {
        if (!raw.ok) throw new Error(String(raw.status));
        const response = (await raw.json()) as ProfileResponse;
        if (!cancelled) {
          setActionCount(
            response.profile
              ? decisionsNeedingAction(response.profile).length
              : 0,
          );
        }
      })
      .catch(() => {
        if (!cancelled) setActionCount(0);
      });
    return () => {
      cancelled = true;
    };
  }, [signedIn, pathname]);
  const actionLabel = `${actionCount} profile review decision${actionCount === 1 ? " needs" : "s need"} your attention`;
  const actionBadge =
    actionCount > 0 ? (
      <span className="nav-badge" title={actionLabel} data-testid="nav-action-badge">
        <span aria-hidden="true">{actionCount}</span>
        <span className="visually-hidden">{actionLabel}</span>
      </span>
    ) : null;

  // Close open menus after navigation.
  useEffect(() => {
    setMoreOpen(false);
    setMobileOpen(false);
  }, [pathname]);

  // Close More on outside click.
  useEffect(() => {
    if (!moreOpen) return;
    function onPointerDown(event: MouseEvent | TouchEvent) {
      if (!moreRef.current) return;
      if (!moreRef.current.contains(event.target as Node)) {
        setMoreOpen(false);
      }
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("touchstart", onPointerDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("touchstart", onPointerDown);
    };
  }, [moreOpen]);

  // Close the mobile menu on outside click.
  useEffect(() => {
    if (!mobileOpen) return;
    function onPointerDown(event: MouseEvent | TouchEvent) {
      if (!mobileRef.current) return;
      if (!mobileRef.current.contains(event.target as Node)) {
        setMobileOpen(false);
      }
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("touchstart", onPointerDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("touchstart", onPointerDown);
    };
  }, [mobileOpen]);

  // Escape closes an open menu and returns focus to its button.
  useEffect(() => {
    if (!moreOpen && !mobileOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (moreOpen) {
        setMoreOpen(false);
        moreButtonRef.current?.focus();
      } else if (mobileOpen) {
        setMobileOpen(false);
        mobileButtonRef.current?.focus();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [moreOpen, mobileOpen]);

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

  // Focus-leave close: when focus moves outside the More element, close it.
  // The deferred check also covers focus moving to the browser chrome, where
  // relatedTarget is null.
  function onMoreBlur(event: React.FocusEvent<HTMLDivElement>) {
    const next = event.relatedTarget as Node | null;
    if (next && moreRef.current?.contains(next)) return;
    window.setTimeout(() => {
      const active = document.activeElement;
      if (!active || !moreRef.current?.contains(active)) {
        setMoreOpen(false);
      }
    }, 0);
  }

  const role = roleOf(session);
  const secondaryItems: NavItem[] = [
    ...SECONDARY_ITEMS,
    ...(canUpload(role) ? [UPLOAD_ITEM] : []),
    ...(role === "admin" ? ADMIN_ITEMS : []),
  ];
  const allItems: NavItem[] = [...PRIMARY_ITEMS, ...secondaryItems];
  const currentInMore = matchesAny(pathname, secondaryItems);

  function renderLink(item: NavItem) {
    const current = isCurrent(pathname, item.href);
    return (
      <Link
        key={item.href}
        href={item.href}
        aria-current={current ? "page" : undefined}
        onClick={() => {
          setMoreOpen(false);
          setMobileOpen(false);
        }}
      >
        <NavIcon name={item.icon} />
        <span>{item.label}</span>
        {item.href === "/profile" ? actionBadge : null}
      </Link>
    );
  }

  return (
    <div className="experience">
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>
      <header className="site-header" data-testid="site-header">
        <div className="site-header-inner">
          <Link href="/genealogy" className="brand">
            TRẦN FAMILY <small>Family archive</small>
          </Link>
          <nav
            className="site-header-nav"
            aria-label="Primary navigation"
            data-testid="primary-nav"
          >
            {PRIMARY_ITEMS.map((item) => renderLink(item))}
            <div className="more" ref={moreRef} onBlur={onMoreBlur}>
              <button
                type="button"
                className="more-button"
                ref={moreButtonRef}
                aria-expanded={moreOpen}
                aria-controls="more-menu"
                aria-haspopup="true"
                aria-label={
                  (currentInMore ? "More, current page" : "More") +
                  (actionCount > 0 ? `, ${actionLabel}` : "")
                }
                data-current={currentInMore ? "true" : undefined}
                onClick={() => {
                  setMobileOpen(false);
                  setMoreOpen((open) => !open);
                }}
                data-testid="more-toggle"
              >
                <span>More</span>
                {currentInMore ? (
                  <span className="current-dot" aria-hidden="true" />
                ) : null}
                {actionBadge}
                <span className="more-caret" aria-hidden="true">
                  ▾
                </span>
              </button>
              {moreOpen ? (
                <ul
                  id="more-menu"
                  className="nav-menu more-panel"
                  data-testid="more-menu"
                >
                  {secondaryItems.map((item) => (
                    <li key={item.href}>{renderLink(item)}</li>
                  ))}
                </ul>
              ) : null}
            </div>
          </nav>
          <div className="site-header-account">
            {session ? (
              <AccountMenu
                session={session}
                signingOut={signingOut}
                onSignIn={signIn}
                onSignOut={signOut}
              />
            ) : null}
          </div>
          <div className="mobile-menu" ref={mobileRef}>
            <button
              type="button"
              className="more-button"
              ref={mobileButtonRef}
              aria-expanded={mobileOpen}
              aria-controls="site-nav"
              aria-haspopup="true"
              onClick={() => {
                setMoreOpen(false);
                setMobileOpen((open) => !open);
              }}
              data-testid="nav-toggle"
            >
              {mobileOpen ? "Close" : "Menu"}
              {mobileOpen ? null : actionBadge}
            </button>
            {mobileOpen ? (
              <nav
                id="site-nav"
                className="mobile-panel"
                aria-label="Main navigation"
                data-testid="site-nav"
              >
                <ul className="nav-menu">
                  {allItems.map((item) => (
                    <li key={item.href}>{renderLink(item)}</li>
                  ))}
                </ul>
              </nav>
            ) : null}
          </div>
        </div>
      </header>
      <main id="main-content" className="app-shell" tabIndex={-1}>
        {children(role, session)}
      </main>
    </div>
  );
}
