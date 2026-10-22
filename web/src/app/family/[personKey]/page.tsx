"use client";

import { useParams } from "next/navigation";

import { ExperienceShell } from "@/components/ExperienceShell";
import { FamilyProfileView } from "@/components/FamilyProfileView";

// Family-facing profile route. The person key is a path parameter; the API
// still decides whether this caller may read it.
export default function FamilyProfilePage() {
  const params = useParams<{ personKey: string }>();
  const raw = params?.personKey;
  const personKey = Array.isArray(raw) ? raw[0] : raw ?? "";
  return (
    <ExperienceShell>
      {/* Keying by person remounts the view on a same-route switch, so the
          previous person's profile can never survive into the new render. */}
      {() => <FamilyProfileView key={personKey} personKey={personKey} />}
    </ExperienceShell>
  );
}
