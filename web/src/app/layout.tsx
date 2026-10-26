import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Trần family archive",
  description:
    "Family tree, cited archive answers, and Head of Family records.",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
