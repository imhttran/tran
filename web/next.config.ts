import type { NextConfig } from "next";

// Same-origin only, mirroring the inventory reference: the browser talks only to
// Next.js, and /api/* is proxied server-side by the authenticated route handler
// in src/app/api/[...path]/route.ts. API_URL stays server-side; the API is never
// exposed to the browser directly (no CORS).
//
// There is intentionally NO `rewrites()` entry for /api/*. An unrestricted
// rewrite would forward the browser's request (and any client-supplied identity
// headers) straight to the Axum API, bypassing the server-side authentication,
// CSRF checks, and identity-header stripping the proxy performs. All /api/*
// traffic lands on the proxy route handler instead.
const nextConfig: NextConfig = {
  // A production build can use a separate output dir (NEXT_DIST_DIR) so it never
  // clobbers the running dev server's .next/ cache.
  distDir: process.env.NEXT_DIST_DIR || ".next",
};

export default nextConfig;
