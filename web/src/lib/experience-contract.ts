// Browser client contract for the Phase 4 experience surfaces.
//
// The browser never makes an authorization decision. Rust (the Axum API, reached
// through the same-origin proxy) is the only authority. This module models its
// answers as a discriminated result so a view can render allowed, denied,
// expired, and no-access without guessing. It also holds the allowlist of
// response fields the browser may receive and a redaction helper that strips
// anything else (account identifiers, OIDC credentials, database URLs).

export type Role = "viewer" | "editor" | "admin";

export type OutcomeKind = "allowed" | "denied" | "expired" | "no_access";

export type Outcome<T> =
  | { kind: "allowed"; data: T }
  | { kind: "denied"; reason: string }
  | { kind: "expired"; signInUrl: string }
  | { kind: "no_access"; reason: string };

// Session surface. It deliberately carries only what the account menu renders:
// the role and a display label chosen by the server. It never carries a subject,
// email, account id, token, OIDC credential, or database URL.
export type SessionView =
  | { state: "anonymous" }
  | { state: "signed_in"; role: Role; display_name: string };

export type SessionResult = Outcome<SessionView>;

// Search and genealogy record surfaces.
export type EvidenceStatus = "verified" | "unverified" | "needs_review";

export type EvidenceRef = {
  evidence_id: string;
  citation: string;
  // Present on genealogy citations; absent when only a citation label exists.
  document_ref?: string;
  page?: number;
};

export type SearchHit = {
  record_id: string;
  title: string;
  corpus: string;
  evidence: EvidenceRef[];
  evidence_status: EvidenceStatus;
};

export type GenealogyRecord = {
  record_id: string;
  person_label: string;
  relationship: string;
  evidence: EvidenceRef[];
  evidence_status: EvidenceStatus;
};

// Head of Family view. Every conclusion links to cited evidence by identifier.
// No field derives a family head from a model answer: a head is only present
// when Rust recorded an appointment backed by verified evidence.
export type HeadOfFamilyConclusion = {
  conclusion: string;
  record_id?: string;
  appointment_id?: string;
  nomination_id?: string;
  head_label: string | null;
  evidence: EvidenceRef[];
  evidence_status: EvidenceStatus;
};

export type HeadOfFamilyView =
  | { catalog: "available"; conclusions: HeadOfFamilyConclusion[] }
  | { catalog: "unavailable"; reason: string };

export type HeadOfFamilyResult = Outcome<HeadOfFamilyView>;

export type NominationRequest = { record_id: string; candidate_label: string };
export type ApprovalRequest = { nomination_id: string };
export type RevocationRequest = { appointment_id: string; reason: string };

export type CommandResult = Outcome<{ ok: true }>;

// Fields the browser may receive from the API. Anything outside this set is
// dropped before it reaches a component.
const ALLOWED_FIELDS = new Set<string>([
  "state",
  "role",
  "display_name",
  "record_id",
  "title",
  "corpus",
  "person_label",
  "relationship",
  "conclusion",
  "record_id",
  "appointment_id",
  "nomination_id",
  "head_label",
  "evidence",
  "evidence_id",
  "citation",
  "document_ref",
  "page",
  "evidence_status",
  "catalog",
  "reason",
  "conclusions",
  "nomination_id",
  "appointment_id",
  "ok",
  "kind",
  "data",
]);

// Field names that must never leave a response toward the browser. A response
// that carries one is treated as a contract violation.
const FORBIDDEN_FIELD_PATTERNS = [
  /password/i,
  /secret/i,
  /token/i,
  /credential/i,
  /client_secret/i,
  /database_url/i,
  /db_url/i,
  /postgres/i,
  /oidc/i,
  /issuer/i,
  /jwks/i,
  /email/i,
  /subject/i,
  /account/i,
  /identity/i,
];

export function isForbiddenField(name: string): boolean {
  return FORBIDDEN_FIELD_PATTERNS.some((pattern) => pattern.test(name));
}

// Drops every field that is not allowlisted or that matches a forbidden pattern.
// Returns a new object; the input is not mutated.
export function redact<T>(value: T): T {
  return redactValue(value) as T;
}

function redactValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(redactValue);
  }
  if (value && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(value)) {
      if (!ALLOWED_FIELDS.has(key) || isForbiddenField(key)) {
        continue;
      }
      result[key] = redactValue(child);
    }
    return result;
  }
  return value;
}

// Maps an HTTP status from the proxy/API onto the discriminated result.
export function outcomeFromStatus<T>(
  status: number,
  payload: T | null,
): Outcome<T> {
  if (status === 401) {
    return { kind: "expired", signInUrl: "/signin" };
  }
  if (status === 403) {
    return { kind: "denied", reason: "Your role does not permit this request." };
  }
  if (status === 404) {
    return { kind: "no_access", reason: "No record is available to your role." };
  }
  if (status >= 200 && status < 300 && payload !== null) {
    return { kind: "allowed", data: redact(payload) };
  }
  return { kind: "denied", reason: `The API returned HTTP ${status}.` };
}

// The OIDC identity provider does not exist yet. Sign-in surfaces that call this
// receive an explicit unsupported result; they must never fabricate a session.
export const OIDC_SUPPORTED = false;

export function unsupportedSignIn(): SessionResult {
  return {
    kind: "no_access",
    reason: "Sign-in is not configured. An operator must supply an OIDC identity provider.",
  };
}

export function canUpload(role: Role | null): boolean {
  return role === "editor" || role === "admin";
}

export function canNominate(role: Role | null): boolean {
  return role === "editor" || role === "admin";
}

export function canApproveOrRevoke(role: Role | null): boolean {
  return role === "admin";
}

export function evidenceStatusLabel(status: EvidenceStatus): string {
  if (status === "unverified") return "Unverified";
  if (status === "needs_review") return "Needs review";
  return "Verified";
}
