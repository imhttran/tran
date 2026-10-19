import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "RAG document Q&A",
  description: "Local RAG web application backed by the Rust Axum API.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
