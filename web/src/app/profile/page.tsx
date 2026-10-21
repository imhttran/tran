"use client";

import { ExperienceShell } from "@/components/ExperienceShell";
import { MyProfile } from "@/components/MyProfile";

// Authenticated My Profile surface. The shell resolves the signed-in session and
// the component renders only what the owner-scoped Rust API returns. Every
// write is re-authorized by Rust; the browser role is presentation only.
export default function ProfilePage() {
  return (
    <ExperienceShell>
      {(role, session) => <MyProfile role={role} session={session} />}
    </ExperienceShell>
  );
}
