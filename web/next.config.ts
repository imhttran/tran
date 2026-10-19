import type { NextConfig } from "next";

// Same-origin only, mirroring the inventory reference: the browser talks only to
// Next.js, and /api/* is proxied server-side to the Rust API. API_URL stays
// server-side; the API is never exposed to the browser directly (no CORS).
const nextConfig: NextConfig = {
  // A production build can use a separate output dir (NEXT_DIST_DIR) so it never
  // clobbers the running dev server's .next/ cache.
  distDir: process.env.NEXT_DIST_DIR || ".next",
  async rewrites() {
    return [
      {
        source: "/api/:path*",
        destination: `${process.env.API_URL || "http://127.0.0.1:8080"}/api/:path*`,
      },
    ];
  },
};

export default nextConfig;
