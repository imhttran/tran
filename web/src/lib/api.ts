// Same-origin API client. The browser only calls the Next.js server, whose
// rewrite proxies `/api/*` to the Rust Axum API. `API_URL` stays server-side.

import type { EvidenceRef, Role } from "./experience-contract";
import {
  SESSION_EXPIRED_MESSAGE,
  noteAuthStatus,
  sessionIsExpired,
} from "./session-expiry";

export type HealthResponse = {
  status: "ok" | "degraded";
  service: string;
  database: "ok" | "down";
  model: "ok" | "down";
};

export type Source = {
  source: string;
  section: string;
  page: number | null;
  text: string;
};

export type IngestResponse = {
  ok: true;
  source: string;
  chunks: number;
  incomplete_pages: number[];
  unchanged: boolean;
};

export type SourceSection = {
  section: string;
  chunk_index: number;
  page: number | null;
};

export type StatusResponse = {
  ok: true;
  source: string;
  corpus: string;
  chunks: number;
  content_hash: string;
  ingested_at: string;
  sections: SourceSection[];
};

// A name question that the API answered from the genealogy store instead of
// document retrieval. `match` describes how the asked name was matched, so the
// view can say that an accent-folded or reordered name is a candidate, not
// proof of identity.
export type PersonMatchKind = "exact" | "alias" | "reordered" | "partial";

export type PersonEvidenceStatus = "verified" | "needs_review" | "unverified";

export type PersonRecordedParent = {
  person_key: string;
  display_name: string;
  evidence_status: PersonEvidenceStatus;
};

export type PersonCandidate = {
  person_key: string;
  display_name: string;
  generation: number | null;
  evidence_status: PersonEvidenceStatus;
  match: PersonMatchKind | null;
  matched_name: string | null;
  profile_url: string;
  recorded_parents: PersonRecordedParent[];
};

export type AssertionAuthority = {
  decision_id: number;
  actor: string;
  reason: string;
  created_at: string;
};

export type PersonAssertion = {
  assertion_id: number;
  kind: string;
  key: string;
  value_text: string | null;
  review_status: string;
  evidence: EvidenceRef[];
  // Present when an authority decision made this assertion the current one.
  // Authoritative is not verified: the two labels are independent.
  authority?: AssertionAuthority | null;
};

export type PersonSelection = {
  person_key: string;
  display_name: string;
  generation: number | null;
  evidence_status: PersonEvidenceStatus;
  match: PersonMatchKind | null;
  matched_name: string | null;
  profile_url: string;
  name_only: boolean;
  assertions: PersonAssertion[];
};

export type PersonAnswer = {
  outcome: "found" | "name_only" | "ambiguous" | "none";
  catalog: "available" | "empty";
  query_name: string;
  match: PersonMatchKind | null;
  truncated: boolean;
  candidates: PersonCandidate[];
  candidates_truncated: boolean;
  selected: PersonSelection | null;
};

export type QueryResponse = {
  answer: string;
  insufficient: boolean;
  sources: Source[];
  person?: PersonAnswer | null;
};

export type ApiErrorBody = {
  ok: false;
  error: string;
  message: string;
  incomplete_pages?: number[];
};

export class ApiRequestError extends Error {
  status: number;
  body: ApiErrorBody | null;

  constructor(status: number, body: ApiErrorBody | null, message: string) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.body = body;
  }
}

function csrfToken(): string | null {
  if (typeof document === "undefined") {
    return null;
  }
  const entry = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith("rag_csrf="));
  return entry ? decodeURIComponent(entry.slice("rag_csrf=".length)) : null;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  // Once this page has observed an expired session, stop issuing protected
  // requests. This ends polling and retry loops instead of replaying an expired
  // credential. The public health probe still works.
  if (path !== "/api/health" && sessionIsExpired()) {
    throw new ApiRequestError(
      401,
      {
        ok: false,
        error: "expired",
        message: SESSION_EXPIRED_MESSAGE,
      },
      SESSION_EXPIRED_MESSAGE,
    );
  }
  const method = (init?.method || "GET").toUpperCase();
  const headers = new Headers(init?.headers);
  if (method !== "GET" && method !== "HEAD") {
    const token = csrfToken();
    if (token) {
      headers.set("x-csrf-token", token);
    }
  }
  let response: Response;
  try {
    response = await fetch(path, { ...init, headers });
  } catch (error) {
    throw new ApiRequestError(
      0,
      null,
      `Could not reach the local API (${error instanceof Error ? error.message : "network error"}).`,
    );
  }
  // Only an authentication rejection expires the browser session; 403, 5xx, and
  // network failures keep their existing behavior.
  noteAuthStatus(response.status);
  const text = await response.text();
  let body: unknown = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!response.ok) {
    const errorBody =
      body && typeof body === "object" && "error" in body
        ? (body as ApiErrorBody)
        : null;
    throw new ApiRequestError(
      response.status,
      errorBody,
      errorBody?.message || `The API returned HTTP ${response.status}.`,
    );
  }
  return body as T;
}

export function getHealth(): Promise<HealthResponse> {
  return request<HealthResponse>("/api/health");
}

export function ingestDocument(file: File): Promise<IngestResponse> {
  const form = new FormData();
  form.append("file", file);
  return request<IngestResponse>("/api/documents/ingest", {
    method: "POST",
    body: form,
  });
}

export function getStatus(source: string): Promise<StatusResponse> {
  return request<StatusResponse>(
    `/api/documents/${encodeURIComponent(source)}/status`,
  );
}

export type QuerySuggestionsResponse = {
  ok: true;
  questions: string[];
};

/// Fetches suggested questions derived from approved archive sections.
/// An empty array means there is no searchable approved content.
export function getQuerySuggestions(
  signal?: AbortSignal,
): Promise<QuerySuggestionsResponse> {
  return request<QuerySuggestionsResponse>("/api/query/suggestions", {
    cache: "no-store",
    signal,
  });
}

export function askQuestion(
  question: string,
  personKey?: string,
): Promise<QueryResponse> {
  const body: { question: string; person_key?: string } = { question };
  if (personKey) {
    body.person_key = personKey;
  }
  return request<QueryResponse>("/api/query", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

// Bulk onboarding (admin only). Every operation goes through the authenticated
// same-origin proxy; mutations carry the CSRF token added by request().
export type BulkClassification =
  | "eligible_for_invite"
  | "needs_review"
  | "already_linked"
  | "already_prepared"
  | "invited"
  | "not_eligible";

export type BulkPreviewRow = {
  person_key: string;
  display_name: string;
  // The verified contact. The server sends only a masked address; the full
  // address never leaves the server. An older payload may omit the field, so
  // the page treats a missing value as `none`.
  contact?: { type: "email"; masked: string } | { type: "none" };
  classification: BulkClassification;
  reasons: string[];
  proposed_username: string | null;
  preparable: boolean;
  blocked_missing_recipient: boolean;
  sendable: boolean;
};

export type BulkPreview = {
  ok: true;
  rows: BulkPreviewRow[];
  summary: {
    eligible_for_invite: number;
    needs_review: number;
    already_linked: number;
    already_prepared: number;
    invited: number;
    not_eligible: number;
    blocked_missing_recipient: number;
  };
  review_needed: string | null;
};

export type BulkPrepareItem = {
  person_key: string;
  status: string;
  reason: string | null;
  reservation_id: number | null;
};

export type BulkPrepare = {
  ok: true;
  batch_id: number;
  batch_key: string;
  created: boolean;
  prepared: number;
  skipped: number;
  needs_review: number;
  failed: number;
  items: BulkPrepareItem[];
};

export type BulkBatchItem = {
  person_key: string;
  display_name: string;
  status: string;
  reason: string | null;
  delivery_status: string;
  provider: string | null;
  attempts: number;
  username: string | null;
  invitation_status: string | null;
  has_verified_contact: boolean;
};

export type BulkBatchStatus = {
  ok: true;
  batch_id: number;
  status: string;
  confirmed_count: number;
  created_by: string;
  created_at: string;
  completed_at: string | null;
  items: BulkBatchItem[];
};

export type BulkSendResult = {
  ok: true;
  configured: true;
  results: Array<{
    person_key: string;
    status: string;
    provider: string | null;
    reason: string | null;
  }>;
};

export function bulkPreview(body: {
  selector: "all" | "all_eligible" | "explicit";
  person_keys?: string[];
  limit?: number;
  offset?: number;
}): Promise<BulkPreview> {
  return request<BulkPreview>("/api/admin/family-members/bulk/preview", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function bulkPrepare(body: {
  person_keys: string[];
  confirmed_count: number;
}): Promise<BulkPrepare> {
  return request<BulkPrepare>("/api/admin/family-members/bulk/prepare", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function bulkVerifyRecipient(body: {
  person_key: string;
  email: string;
  confirmed: true;
}): Promise<{ ok: true; verified: true }> {
  return request<{ ok: true; verified: true }>(
    "/api/admin/family-members/bulk/recipients",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
}

export function bulkSend(body: {
  batch_id: number;
  person_keys?: string[];
  approve: true;
  ttl_secs?: number;
}): Promise<BulkSendResult> {
  return request<BulkSendResult>("/api/admin/family-members/bulk/send", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function bulkBatchStatus(batchId: number): Promise<BulkBatchStatus> {
  return request<BulkBatchStatus>(
    `/api/admin/family-members/bulk/batches/${batchId}`,
  );
}

// --- profile and private media --------------------------------------------

export type ProfileVisibility = "owner_private" | "family";
export type ProfileStatus =
  "draft" | "pending_review" | "approved" | "rejected" | "needs_changes";

export type ApprovedProfileValue = {
  field_key: string;
  value: string;
  visibility: ProfileVisibility;
  approved_revision_number: number;
  approved_revision_id: number | null;
  updated_at: string;
  /** An active owner restriction keeps this family value off family pages. */
  restricted_from_family: boolean;
  restriction_generation: number | null;
};

export type ProfileRevision = {
  revision_id: number;
  field_key: string;
  revision_number: number;
  prior_value_text: string | null;
  prior_approved_revision_number: number;
  value: string;
  visibility: ProfileVisibility;
  status: ProfileStatus;
  changed_at: string;
  submitted_at: string | null;
  reviewed_at: string | null;
  review_note: string | null;
};

export type ProfileMediaAsset = {
  media_id: number;
  revision_id: number;
  kind: "profile_photo" | "personal_document";
  mime_type: string;
  byte_size: number;
  file_name: string;
  visibility: ProfileVisibility;
  width: number | null;
  height: number | null;
  status: ProfileStatus;
  created_at: string;
  /// True when an active A2 restriction withholds this approved family-visible
  /// asset from family pages. Present on the owner profile's `media` entries.
  restricted_from_family?: boolean;
};

export type ProfileResponse = {
  ok: true;
  linked: boolean;
  profile?: {
    display_name: string;
    approved: ApprovedProfileValue[];
    pending: ProfileRevision[];
    history: ProfileRevision[];
  };
  media?: ProfileMediaAsset[];
};

export function getProfile(): Promise<ProfileResponse> {
  return request<ProfileResponse>("/api/profile");
}

export function createProfileDraft(input: {
  field_key: string;
  value: string;
  visibility: ProfileVisibility;
}): Promise<{ ok: true; revision: ProfileRevision }> {
  return request("/api/profile/drafts", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
}

export function updateProfileDraft(
  revisionId: number,
  input: { value: string; visibility: ProfileVisibility },
): Promise<{ ok: true; revision: ProfileRevision }> {
  return request(`/api/profile/drafts/${revisionId}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
}

export function submitProfileDraft(
  revisionId: number,
): Promise<{ ok: true; revision: ProfileRevision }> {
  return request(`/api/profile/drafts/${revisionId}/submit`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({}),
  });
}

export function cancelProfileDraft(
  revisionId: number,
): Promise<{ ok: true; revision: ProfileRevision }> {
  return request(`/api/profile/drafts/${revisionId}/cancel`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({}),
  });
}

export function uploadProfileMedia(
  kind: "photo" | "document",
  file: File,
  visibility: ProfileVisibility,
): Promise<{ ok: true; media: ProfileMediaAsset }> {
  const form = new FormData();
  form.append("file", file);
  form.append("visibility", visibility);
  const path =
    kind === "photo"
      ? "/api/profile/media/photo"
      : "/api/profile/media/personal-document";
  return request(path, { method: "POST", body: form });
}

export function withdrawProfileMedia(
  mediaId: number,
): Promise<{ ok: true; withdrawn: true }> {
  return request(`/api/profile/media/${mediaId}`, { method: "DELETE" });
}

export function profileMediaUrl(mediaId: number): string {
  return `/api/profile/media/${mediaId}`;
}

// --- administrator review console -----------------------------------------

export type ReviewQueueStatus =
  "pending_review" | "approved" | "rejected" | "needs_changes" | "all";

export type ReviewMedia = {
  media_id: number;
  kind: string;
  mime_type: string | null;
  file_name: string | null;
  byte_size: number | null;
  width: number | null;
  height: number | null;
};

export type ReviewDecision = "approve" | "reject" | "needs_changes";

export type ReviewEntry = {
  display_name: string;
  revision: ProfileRevision;
  current_approved: ApprovedProfileValue | null;
  current_media: ReviewMedia | null;
  media: ReviewMedia | null;
  can_decide: boolean;
  decision_block:
    "self_submitted" | "reviewer_owns_person" | "not_pending" | null;
};

export type ReviewQueueResponse = {
  ok: true;
  status: string;
  limit: number;
  next_cursor: number | null;
  entries: ReviewEntry[];
};

export type ReviewEvent = {
  event: string;
  actor: string;
  note: string | null;
  created_at: string;
};

export type ReviewDetailResponse = {
  ok: true;
  entry: ReviewEntry;
  events: ReviewEvent[];
  history: ProfileRevision[];
};

export function getReviewQueue(params: {
  status: ReviewQueueStatus;
  cursor?: number | null;
  limit?: number;
}): Promise<ReviewQueueResponse> {
  const search = new URLSearchParams({ status: params.status });
  if (params.cursor) search.set("cursor", String(params.cursor));
  if (params.limit) search.set("limit", String(params.limit));
  return request<ReviewQueueResponse>(`/api/admin/reviews?${search}`);
}

export function getReviewDetail(
  revisionId: number,
): Promise<ReviewDetailResponse> {
  return request<ReviewDetailResponse>(`/api/admin/reviews/${revisionId}`);
}

export function reviewMediaUrl(mediaId: number): string {
  return `/api/admin/reviews/media/${mediaId}`;
}

export function decideProfileRevision(
  revisionId: number,
  decision: ReviewDecision,
  note: string,
): Promise<{ ok: true; revision: ProfileRevision }> {
  return request(`/api/profile/revisions/${revisionId}/review`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ decision, note }),
  });
}

// --- source narrative review (admin) ---------------------------------------

export type NarrativeReviewStatus = "pending" | "approved" | "rejected" | "all";
export type NarrativeLinkStatus =
  "resolved" | "ambiguous" | "confirmed" | "rejected";
export type NarrativeLinkFilter =
  | "all"
  | "needs_confirmation"
  | "no_accepted"
  | "has_accepted"
  | "no_candidates";
export type NarrativeVisibilityFilter = "all" | "admin_only" | "family";
export type NarrativePassageDecision = "approved" | "rejected" | "pending";
export type NarrativePassageVisibility = "family" | "admin_only";

export type NarrativeReviewLink = {
  person_key: string;
  display_name: string;
  link_status: NarrativeLinkStatus;
  match_kind: string;
  evidence_note: string | null;
  reviewed_by: string | null;
  reviewed_at: string | null;
};

export type NarrativeReviewPassage = {
  passage_id: number;
  language: "vi" | "en";
  section_title: string | null;
  document_ref: string;
  page_start: number;
  page_end: number;
  review_status: "pending" | "approved" | "rejected";
  visibility: "admin_only" | "family";
  body_excerpt: string;
  body?: string;
  updated_at: string;
};

export type NarrativeReviewPerson = {
  person_key: string;
  display_name: string;
  aliases: string[];
  birth: string | null;
  death: string | null;
  parents: string[];
  spouses: string[];
  children: string[];
};

export type NarrativeReviewEvent = {
  event_id: number;
  person_key: string | null;
  actor: string;
  action: string;
  from_status: string | null;
  to_status: string;
  visibility: string | null;
  note: string | null;
  created_at: string;
};

export type NarrativeReviewQueueEntry = {
  passage: NarrativeReviewPassage;
  links: NarrativeReviewLink[];
};

export type NarrativeReviewQueueResponse = {
  ok: true;
  status: NarrativeReviewStatus;
  language: "vi" | "en" | null;
  person: string | null;
  unmatched: boolean;
  link: NarrativeLinkFilter;
  visibility: "admin_only" | "family" | null;
  document: string | null;
  page_from: number | null;
  page_to: number | null;
  search: string | null;
  offset: number;
  limit: number;
  total: number;
  truncated: boolean;
  documents: string[];
  entries: NarrativeReviewQueueEntry[];
};

export type NarrativeReviewDetailResponse = {
  ok: true;
  passage: NarrativeReviewPassage;
  links: NarrativeReviewLink[];
  people: NarrativeReviewPerson[];
  events: NarrativeReviewEvent[];
};

export type NarrativeReviewPeopleResponse = {
  ok: true;
  people: NarrativeReviewPerson[];
};

export function searchNarrativeReviewPeople(
  query: string,
  signal?: AbortSignal,
): Promise<NarrativeReviewPeopleResponse> {
  const search = new URLSearchParams({ q: query, limit: "12" });
  return request<NarrativeReviewPeopleResponse>(
    `/api/admin/narrative/persons?${search}`,
    { cache: "no-store", signal },
  );
}

export function getNarrativeReviewQueue(params: {
  status: NarrativeReviewStatus;
  language: "all" | "vi" | "en";
  person?: string;
  unmatched?: boolean;
  link?: NarrativeLinkFilter;
  visibility?: NarrativeVisibilityFilter;
  document?: string;
  pageFrom?: number;
  pageTo?: number;
  search?: string;
  offset?: number;
  limit?: number;
}): Promise<NarrativeReviewQueueResponse> {
  const search = new URLSearchParams({
    status: params.status,
    language: params.language,
  });
  if (params.person?.trim()) search.set("person", params.person.trim());
  if (params.unmatched) search.set("unmatched", "true");
  if (params.link && params.link !== "all") search.set("link", params.link);
  if (params.visibility && params.visibility !== "all") {
    search.set("visibility", params.visibility);
  }
  if (params.document?.trim()) search.set("document", params.document.trim());
  if (params.pageFrom != null) search.set("page_from", String(params.pageFrom));
  if (params.pageTo != null) search.set("page_to", String(params.pageTo));
  if (params.search?.trim()) search.set("search", params.search.trim());
  if (params.offset != null) search.set("offset", String(params.offset));
  if (params.limit) search.set("limit", String(params.limit));
  return request<NarrativeReviewQueueResponse>(
    `/api/admin/narrative/passages?${search}`,
    { cache: "no-store" },
  );
}

export function getNarrativeReviewDetail(
  passageId: number,
): Promise<NarrativeReviewDetailResponse> {
  return request<NarrativeReviewDetailResponse>(
    `/api/admin/narrative/passages/${passageId}`,
    { cache: "no-store" },
  );
}

export function reviewNarrativeLink(
  passageId: number,
  personKey: string,
  decision: "confirm" | "reject",
  expectedStatus: NarrativeLinkStatus,
  note: string,
): Promise<NarrativeReviewDetailResponse> {
  return postJson<NarrativeReviewDetailResponse>(
    `/api/admin/narrative/passages/${passageId}/links`,
    { person_key: personKey, decision, expected_status: expectedStatus, note },
  );
}

export function reviewNarrativePassage(
  passageId: number,
  decision: NarrativePassageDecision,
  visibility: NarrativePassageVisibility,
  expectedStatus: NarrativeReviewPassage["review_status"],
  expectedVersion: string,
  note: string,
): Promise<NarrativeReviewDetailResponse> {
  return postJson<NarrativeReviewDetailResponse>(
    `/api/admin/narrative/passages/${passageId}/review`,
    {
      decision,
      visibility,
      expected_status: expectedStatus,
      expected_version: expectedVersion,
      note,
    },
  );
}

// --- branch-scoped shared-fact review --------------------------------------
//
// Every call is authorized by Rust. The client never sends an account id, a
// reviewer identity, or a branch it merely wants: the server resolves the
// caller from the verified session and checks the explicit branch grant.

export type BranchGrant = {
  branch_key: string;
  branch_label: string | null;
};

export type BranchGrantsResponse = {
  ok: true;
  grants: BranchGrant[];
};

export type BranchProposalStatus =
  "pending_review" | "approved" | "rejected" | "needs_changes";

// The console sends canonical statuses only; the API still accepts the
// `pending` alias for other callers.
export type BranchQueueFilter = BranchProposalStatus | "all";

export type BranchEvidenceRef = {
  source_evidence_id: number;
  document_ref: string;
  page: number;
};

export type BranchProposal = {
  proposal_id: number;
  person_key: string;
  display_name: string;
  branch_key: string;
  assertion_kind: string;
  assertion_key: string;
  prior_value_text: string | null;
  prior_review_status: string | null;
  proposed_value_text: string;
  evidence: BranchEvidenceRef[];
  submitted_at: string;
  status: BranchProposalStatus;
  can_decide: boolean;
  decision_block: string | null;
};

export type BranchQueueResponse = {
  ok: true;
  status: string;
  limit: number;
  next_cursor: number | null;
  submissions: BranchProposal[];
};

export type BranchCurrentAssertion = {
  assertion_id: number;
  value_text: string | null;
  review_status: string;
  reviewed_at: string | null;
};

export type BranchProposalEvent = {
  event: string;
  actor: string;
  note: string | null;
  created_at: string;
};

export type BranchProposalDetailResponse = {
  ok: true;
  proposal: BranchProposal;
  current_assertion: BranchCurrentAssertion | null;
  events: BranchProposalEvent[];
};

export type BranchDecision = "approve" | "reject" | "needs_changes";

export type BranchPersonEvidence = {
  source_evidence_id: number;
  document_ref: string;
  page: number;
  locator: string | null;
  assertion_kind: string;
  assertion_key: string;
};

export function getBranchGrants(): Promise<BranchGrantsResponse> {
  return request<BranchGrantsResponse>("/api/branch-review/grants/me");
}

export function getBranchQueue(params: {
  branch?: string | null;
  status: BranchQueueFilter;
  cursor?: number | null;
  limit?: number;
}): Promise<BranchQueueResponse> {
  const search = new URLSearchParams({ status: params.status });
  if (params.branch) search.set("branch", params.branch);
  if (params.cursor) search.set("cursor", String(params.cursor));
  if (params.limit) search.set("limit", String(params.limit));
  return request<BranchQueueResponse>(
    `/api/branch-review/submissions?${search}`,
  );
}

export function getBranchProposal(
  proposalId: number,
): Promise<BranchProposalDetailResponse> {
  return request<BranchProposalDetailResponse>(
    `/api/branch-review/submissions/${proposalId}`,
  );
}

export function decideBranchProposal(
  proposalId: number,
  decision: BranchDecision,
  note: string,
): Promise<{
  ok: true;
  proposal_id: number;
  status: string;
  applied_assertion_id: number | null;
}> {
  return request(`/api/branch-review/submissions/${proposalId}/decision`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ decision, note }),
  });
}

export function getBranchPersonEvidence(
  personKey: string,
): Promise<{ ok: true; evidence: BranchPersonEvidence[] }> {
  return request(
    `/api/branch-review/persons/${encodeURIComponent(personKey)}/evidence`,
  );
}

// --- Admin > Family Members: accounts -----------------------------------------

export type MemberStatus = "active" | "deactivated";

export type FamilyMember = {
  user_id: number;
  issuer: string;
  subject: string;
  email: string | null;
  display_name: string | null;
  roles: Role[];
  status: MemberStatus;
  created_at: string;
  linked_person: {
    link_id: number;
    person_key: string;
    display_name: string;
    username: string | null;
  } | null;
};

export type FamilyMembersPage = {
  ok: true;
  members: FamilyMember[];
  next_after: number | null;
  linkage: "available" | "unavailable";
};

export type MemberFilter = {
  q: string;
  status: MemberStatus | "";
  role: Role | "";
};

type AccountRef = Pick<FamilyMember, "issuer" | "subject">;

export function listFamilyMembers(
  filter: MemberFilter,
  after: number | null,
): Promise<FamilyMembersPage> {
  const params = new URLSearchParams();
  if (filter.q.trim()) params.set("q", filter.q.trim());
  if (filter.status) params.set("status", filter.status);
  if (filter.role) params.set("role", filter.role);
  if (after) params.set("after", String(after));
  const query = params.toString();
  return request<FamilyMembersPage>(
    `/api/admin/family-members${query ? `?${query}` : ""}`,
    {
      cache: "no-store",
    },
  );
}

function postJson<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function changeMemberRole(
  member: AccountRef,
  role: Role,
  revoke: boolean,
): Promise<{ ok: true }> {
  return postJson("/api/admin/roles", {
    issuer: member.issuer,
    subject: member.subject,
    role,
    revoke,
  });
}

export function deactivateMember(member: AccountRef): Promise<{ ok: true }> {
  return postJson(
    `/api/admin/users/${encodeURIComponent(member.issuer)}/${encodeURIComponent(member.subject)}/deactivate`,
    {},
  );
}

export function requestPasswordReset(
  member: AccountRef,
): Promise<{ ok: true; state: "requested" }> {
  return postJson("/api/admin/family-members/password-reset", {
    issuer: member.issuer,
    subject: member.subject,
  });
}

export function linkMember(
  member: AccountRef,
  personKey: string,
): Promise<{ ok: true; linked: boolean }> {
  return postJson("/api/admin/family-members/links", {
    person_key: personKey,
    issuer: member.issuer,
    subject: member.subject,
    decision: "approve",
  });
}

export function revokeMemberLink(linkId: number): Promise<{ ok: true }> {
  return postJson(`/api/admin/family-members/links/${linkId}/revoke`, {});
}

// Family document lifecycle (FAM-003). Uploads are submissions: they create a
// pending version and write no chunks until an admin approves and processes it.
export type DocumentSubmission = {
  version_id: number;
  document_key: string;
  version_number: number;
  corpus: string;
  source: string;
  title: string;
  description: string | null;
  category: string;
  origin: string;
  access_scope: string | null;
  review_status: string;
  processing_status: string;
  content_sha256: string;
  byte_size: number;
  mime_type: string;
  original_filename: string;
  reviewed_by: string | null;
  reviewed_at: string | null;
  review_note: string | null;
  removed_at: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
};

export type DocumentSubmissionResult = {
  ok: true;
  version_id: number;
  review_status: string;
  processing_status: string;
  chunks: number;
};

export type DocumentSubmissionList = {
  ok: true;
  submissions: DocumentSubmission[];
};

export function submitDocument(
  form: FormData,
): Promise<DocumentSubmissionResult> {
  return request<DocumentSubmissionResult>("/api/documents/submissions", {
    method: "POST",
    body: form,
  });
}

export function listDocumentSubmissions(): Promise<DocumentSubmissionList> {
  return request<DocumentSubmissionList>("/api/documents/submissions");
}

// Reviewed document-to-genealogy extraction. Draft proposals are admin-only;
// applying one writes unverified genealogy assertions and never verifies them.
export type ExtractionCandidate = {
  person_key: string;
  display_name: string;
  evidence_status: string;
  match_kind: string;
  matched_name: string;
};

export type ExtractionProposal = {
  proposal_id: number;
  ordinal: number;
  kind: "person" | "parent_child" | "spouse" | "alias" | "social_link";
  subject_name: string;
  target_name: string | null;
  resolution: "matched" | "ambiguous" | "unresolved" | "create";
  resolved_person_key: string | null;
  candidates: ExtractionCandidate[];
  value_text: string | null;
  support_text: string;
  status: "proposed" | "approved" | "rejected" | "applied";
  decided_by: string | null;
  applied_result: unknown;
};

export type DocumentExtraction = {
  ok: true;
  extraction: {
    extraction_id: number;
    document_version_id: number;
    document_source: string;
    content_sha256: string;
    status: "draft" | "applied" | "superseded";
    created_by: string;
    created_at: string;
    applied_by: string | null;
    applied_at: string | null;
  };
  proposals: ExtractionProposal[];
};

export type ExtractionApplyResult = {
  ok: true;
  extraction_id: number;
  already_applied: boolean;
  applied: Array<{ proposal_id: number; kind: string; result: unknown }>;
};

export async function getDocumentExtraction(
  versionId: number,
): Promise<DocumentExtraction | null> {
  try {
    return await request<DocumentExtraction>(
      `/api/genealogy/documents/${versionId}/extraction`,
    );
  } catch (error) {
    if (error instanceof ApiRequestError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export function extractDocumentRecords(
  versionId: number,
): Promise<DocumentExtraction> {
  return request<DocumentExtraction>(
    `/api/genealogy/documents/${versionId}/extraction`,
    { method: "POST" },
  );
}

export function decideExtractionProposal(
  proposalId: number,
  body: {
    decision: "approve" | "reject";
    person_key?: string;
    create_person?: boolean;
    value_text?: string;
  },
): Promise<DocumentExtraction> {
  return request<DocumentExtraction>(
    `/api/genealogy/document-proposals/${proposalId}/decision`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
}

export function applyDocumentExtraction(
  versionId: number,
): Promise<ExtractionApplyResult> {
  return request<ExtractionApplyResult>(
    `/api/genealogy/documents/${versionId}/apply`,
    { method: "POST" },
  );
}

// --- authoritative source decisions --------------------------------------

export type AuthorityAction = "added" | "replaced" | "removed" | "unchanged";

export type AuthorityCitation = {
  document_ref: string;
  page: number;
  locator: string | null;
  excerpt: string | null;
  label: string;
  // Present only in the authorized history view: the source was removed.
  withdrawn?: boolean;
};

export type AuthorityAssertion = {
  assertion_id: number;
  person_key: string;
  display_name: string;
  kind: string;
  key: string;
  value_text: string | null;
  review_status: string;
  citations: AuthorityCitation[];
};

export type AuthorityItem = {
  action: AuthorityAction;
  incoming: AuthorityAssertion | null;
  replaces: AuthorityAssertion[];
};

export type AuthorityConflict = {
  code: string;
  assertion: AuthorityAssertion;
};

export type AuthorityPreview = {
  document_version_id: number;
  document_ref: string;
  content_sha256: string;
  scope: "document" | "person" | "assertions";
  person_key: string | null;
  items: AuthorityItem[];
  omissions: AuthorityAssertion[];
  conflicts: AuthorityConflict[];
  preview_hash: string;
};

export type AuthorityDecision = {
  decision_id: number;
  scope: "document" | "person" | "assertions";
  kind: "designate" | "reversal";
  status: "active" | "superseded" | "reversed";
  document_version_id: number;
  document_ref: string;
  content_sha256: string;
  person_key: string | null;
  reason: string;
  actor: string;
  created_at: string;
  retired_at: string | null;
  reverses_decision_id: number | null;
  supersedes_decision_id: number | null;
  items: AuthorityItem[];
  // True when this active decision made an assertion authoritative and every
  // citation for it has since been withdrawn. The decision stays; the reviewer
  // must reverse it or designate another source.
  needs_attention: boolean;
  unsupported_assertions: AuthorityAssertion[];
};

export type AuthorityScopeBody = {
  scope: "document" | "person" | "assertions";
  person_key?: string;
  assertion_ids?: number[];
  remove_assertion_ids?: number[];
};

export function previewDocumentAuthority(
  versionId: number,
  body: AuthorityScopeBody,
): Promise<{ ok: true; preview: AuthorityPreview }> {
  return request<{ ok: true; preview: AuthorityPreview }>(
    `/api/genealogy/documents/${versionId}/authority/preview`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
}

export function applyDocumentAuthority(
  versionId: number,
  body: AuthorityScopeBody & {
    reason: string;
    preview_hash: string;
    confirm: boolean;
    acknowledge_conflicts: boolean;
  },
): Promise<{
  ok: true;
  decision: {
    decision_id: number;
    status: string;
    applied: number;
    superseded: number;
    removed: number;
  };
}> {
  return request<{
    ok: true;
    decision: {
      decision_id: number;
      status: string;
      applied: number;
      superseded: number;
      removed: number;
    };
  }>(`/api/genealogy/documents/${versionId}/authority/apply`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function listAuthorityDecisions(
  personKey?: string,
): Promise<{ ok: true; decisions: AuthorityDecision[] }> {
  const query = personKey ? `?person_key=${encodeURIComponent(personKey)}` : "";
  return request<{ ok: true; decisions: AuthorityDecision[] }>(
    `/api/genealogy/authority${query}`,
  );
}

export function reverseAuthorityDecision(
  decisionId: number,
  reason: string,
): Promise<{
  ok: true;
  decision: { decision_id: number; status: string };
}> {
  return request<{
    ok: true;
    decision: { decision_id: number; status: string };
  }>(`/api/genealogy/authority/${decisionId}/reverse`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ reason }),
  });
}

export function reviewDocument(
  versionId: number,
  body: { approve: boolean; note?: string; scope?: string },
): Promise<{ ok: true; submission: DocumentSubmission }> {
  return request<{ ok: true; submission: DocumentSubmission }>(
    `/api/documents/submissions/${versionId}/review`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
}

export function changeDocumentScope(
  versionId: number,
  scope: "family" | "admin_only",
): Promise<{ ok: true; submission: DocumentSubmission }> {
  return request<{ ok: true; submission: DocumentSubmission }>(
    `/api/documents/submissions/${versionId}/scope`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ scope }),
    },
  );
}

export function removeDocument(
  versionId: number,
): Promise<{ ok: true; submission: DocumentSubmission }> {
  return request<{ ok: true; submission: DocumentSubmission }>(
    `/api/documents/submissions/${versionId}/remove`,
    { method: "POST" },
  );
}

export function processDocument(versionId: number): Promise<{
  ok: true;
  job_id?: number;
  processing_status?: string;
  promoted?: boolean;
}> {
  return request(`/api/documents/submissions/${versionId}/process`, {
    method: "POST",
  });
}

export type LegacyDocumentInventoryRow = {
  corpus: string;
  source: string;
  version_id: number;
  chunks: number;
  review_status: string;
  processing_status: string;
  fingerprint: string;
};

export function legacyDocumentInventory(): Promise<{
  ok: true;
  inventory: LegacyDocumentInventoryRow[];
}> {
  return request("/api/documents/legacy");
}

export function promoteLegacyDocument(
  versionId: number,
): Promise<{ ok: true; submission: DocumentSubmission }> {
  return request<{ ok: true; submission: DocumentSubmission }>(
    `/api/documents/legacy/${versionId}/promote`,
    { method: "POST" },
  );
}

export type DocumentJob = {
  job_id: number;
  version_id: number;
  status: string;
  attempts: number;
  max_attempts: number;
  last_error: string | null;
  created_at: string;
  updated_at: string;
};

export function runDocumentJobs(): Promise<{
  ok: true;
  processed: Array<{
    job_id: number;
    version_id: number;
    status: string;
    error: string | null;
  }>;
}> {
  return request("/api/documents/jobs/run", { method: "POST" });
}

export function listDocumentJobs(): Promise<{ ok: true; jobs: DocumentJob[] }> {
  return request("/api/documents/jobs");
}

export function retryDocumentJob(
  jobId: number,
): Promise<{ ok: true; job_id: number; version_id: number; status: string }> {
  return request(`/api/documents/jobs/${jobId}/retry`, { method: "POST" });
}

// --- family profile presentation (USR-006-A/B) -----------------------------
//
// Read-only, authorized server-side. The response contains only values whose
// effective visibility is `family`; the browser filters the presented fields
// further (see FamilyProfileView) and never asks for private profile data.

export type FamilyProfileFieldView = {
  field_key: string;
  value: string;
  visibility: string;
  approved_revision_number: number;
  updated_at: string;
};

/// One approved, family-visible media asset. The response carries metadata only;
/// the bytes are fetched from the person-scoped family media route, which applies
/// the same authorization.
export type FamilyProfileMediaView = {
  kind: "profile_photo" | "personal_document";
  media_id: number;
  mime_type: string;
  byte_size: number;
  file_name: string;
  width: number | null;
  height: number | null;
  approved_revision_number: number;
  created_at: string;
};

export type FamilyProfileDateView = {
  kind: "birth_date" | "death_date";
  value: string | null;
  start: string;
  end: string | null;
  precision: "day" | "month" | "year" | "range";
  qualifier: "exact" | "circa" | "before" | "after";
};

/// One approved archive passage. The body is the source wording; the page range
/// is the PDF citation. Pending, rejected, admin-only, and superseded passages
/// never appear in this array.
export type FamilyProfilePassageView = {
  passage_id: number;
  language: "vi" | "en";
  /// The source document's relative path. Absent on an older server payload.
  document_ref?: string | null;
  section_title: string | null;
  page_start: number;
  page_end: number;
  body: string;
};

/// One published archive photograph. The bytes are fetched from the
/// authorization-checked photograph routes; the payload carries no storage key
/// or filesystem path.
export type FamilyArchivePhotographView = {
  asset_id: number;
  caption: string | null;
  mime_type: string;
  byte_size: number;
  width: number | null;
  height: number | null;
  document_version_id: number;
  corpus: string;
  page_number: number;
  /// The PDF page citation, for example "PDF page 4".
  citation: string;
  created_at: string;
  thumbnail_url: string;
  original_url: string;
};

export type GalleryPhotographView = FamilyArchivePhotographView & {
  /// Confirmed person links only. An unlinked photograph has an empty array.
  people: { person_key: string; display_name: string }[];
};

export type PhotographGalleryResponse = {
  ok: true;
  photographs: GalleryPhotographView[];
};

export type AdminArchivePhotographLinkView = {
  person_id: number;
  person_key: string;
  display_name: string;
  status: "proposed" | "confirmed" | "rejected";
  decided_by: string | null;
  decided_at: string | null;
  source_page: number | null;
  supporting_text: string | null;
  confidence: "high" | "medium" | "low" | null;
  confidence_basis: string | null;
  ambiguity_note: string | null;
  match_kind: "exact" | "alias" | "reordered" | "partial" | null;
  candidate_rank: number | null;
  proposal_origin: "caption" | "layout" | "manual" | null;
};

export type AdminArchivePhotographView = {
  asset_id: number;
  caption: string | null;
  publication_state: "unpublished" | "published";
  mime_type: string;
  byte_size: number;
  width: number | null;
  height: number | null;
  object_id: string;
  classification: string;
  classification_rule: string;
  document_version_id: number;
  corpus: string;
  page_numbers: number[];
  citation: string;
  created_at: string;
  thumbnail_url: string;
  original_url: string;
  links: AdminArchivePhotographLinkView[];
};

export type AdminPhotographsResponse = {
  ok: true;
  photographs: AdminArchivePhotographView[];
};

export type PhotographProposalCandidateView = {
  person_id: number;
  person_key: string;
  display_name: string;
  match_kind: "exact" | "alias" | "reordered" | "partial";
  confidence: "high" | "medium" | "low";
  source_kind:
    | "caption"
    | "explicit_group"
    | "section_heading"
    | "nearby_text"
    | "biography"
    | "adjacent_page";
  source_page: number;
  supporting_text: string;
  confidence_basis: string;
  ambiguity: string | null;
  candidate_rank: number;
};

export type PhotographProposalResponse = {
  ok: true;
  candidates: PhotographProposalCandidateView[];
  unlinked_reason: string | null;
  preserved_decisions: number;
};

export type PhotographReviewChangeResponse = {
  ok: true;
  changed: boolean;
  prior?: string;
  next?: string;
  operation?: string;
};

export type FamilyProfileResponse = {
  ok: true;
  state: "published" | "empty";
  person: { person_key: string; display_name: string };
  fields: FamilyProfileFieldView[];
  media: FamilyProfileMediaView[];
  /// Verified birth and death dates. Empty or absent when nothing is verified
  /// yet; the field is additive, so an older response remains valid.
  dates?: FamilyProfileDateView[];
  /// Approved archive history, separate from the member's own profile fields.
  history?: FamilyProfilePassageView[];
  /// Published archive photographs linked to this person. Separate from
  /// `media`, which is owner-uploaded profile media. Absent on an older server
  /// payload, so the view treats it as an empty list.
  archive_photographs?: FamilyArchivePhotographView[];
};

/// The person-scoped history response used by the family-tree dialog. It carries
/// the same published passages as the profile, with the same authorization; the
/// separate route exists so the tree fetches history only when a dialog opens.
export type FamilyHistoryResponse = {
  ok: true;
  person: { person_key: string; display_name: string };
  history: FamilyProfilePassageView[];
};

/// Decision A2: immediately stop an approved family-visible field from being
/// presented to the family. Idempotent; only the owner's own field is reachable.
export function restrictProfileField(fieldKey: string): Promise<{
  ok: true;
  field_key: string;
  generation: number;
  already_restricted: boolean;
  restricted_from_family: boolean;
}> {
  return request(
    `/api/profile/fields/${encodeURIComponent(fieldKey)}/restrict`,
    { method: "POST" },
  );
}

export function getFamilyProfile(
  personKey: string,
): Promise<FamilyProfileResponse> {
  // Private, authorization-sensitive read: never serve it from a cache.
  return request<FamilyProfileResponse>(
    `/api/family/persons/${encodeURIComponent(personKey)}/profile`,
    { cache: "no-store" },
  );
}

/// Fetches the published archive history for one person. Used by the family-tree
/// dialog when it opens; a failure must replace, never preserve, prior content.
export function getFamilyHistory(
  personKey: string,
  signal?: AbortSignal,
): Promise<FamilyHistoryResponse> {
  return request<FamilyHistoryResponse>(
    `/api/family/persons/${encodeURIComponent(personKey)}/history`,
    { cache: "no-store", signal },
  );
}

/// Same-origin asset URL for one approved family-visible media item. Rendered
/// directly in an `<img>` or link, so the server-side proxy forwards the session
/// credential and the Rust API makes the authorization decision.
export function familyProfileMediaUrl(
  personKey: string,
  mediaId: number,
): string {
  return `/api/family/persons/${encodeURIComponent(personKey)}/media/${mediaId}`;
}

// --- archive photographs (AP-106/AP-107) ------------------------------------

export function getPhotographGallery(): Promise<PhotographGalleryResponse> {
  return request<PhotographGalleryResponse>("/api/family/photographs", {
    cache: "no-store",
  });
}

export function familyPhotographThumbnailUrl(assetId: number): string {
  return `/api/family/photographs/${assetId}/thumbnail`;
}

export function familyPhotographOriginalUrl(assetId: number): string {
  return `/api/family/photographs/${assetId}/original`;
}

export function getAdminPhotographs(): Promise<AdminPhotographsResponse> {
  return request<AdminPhotographsResponse>("/api/admin/photographs", {
    cache: "no-store",
  });
}

export function setPhotographVisibility(
  assetId: number,
  visibility: "published" | "unpublished",
): Promise<PhotographReviewChangeResponse> {
  return request<PhotographReviewChangeResponse>(
    `/api/admin/photographs/${assetId}/visibility`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ visibility }),
    },
  );
}

export function updatePhotographCaption(
  assetId: number,
  caption: string,
): Promise<PhotographReviewChangeResponse> {
  return request<PhotographReviewChangeResponse>(
    `/api/admin/photographs/${assetId}/caption`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ caption }),
    },
  );
}

export function proposePhotographLinks(
  assetId: number,
): Promise<PhotographProposalResponse> {
  return request<PhotographProposalResponse>(
    `/api/admin/photographs/${assetId}/propose`,
    { method: "POST" },
  );
}

export function decidePhotographLink(
  assetId: number,
  personId: number,
  decision: "confirm" | "reject",
): Promise<PhotographReviewChangeResponse> {
  return request<PhotographReviewChangeResponse>(
    `/api/admin/photographs/${assetId}/links/${personId}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ decision }),
    },
  );
}

// --- Admin > Family Members: branch reviewer grants (USR-005A) ---------------

export type ReviewerBranch = {
  family_branch_id: number;
  branch_key: string;
  branch_label: string | null;
};

export type ReviewerGrant = ReviewerBranch & {
  grant_id: number;
  account_id: number;
  active: boolean;
  granted_reason: string | null;
};

export function listReviewerBranches(): Promise<{ ok: true; branches: ReviewerBranch[] }> {
  return request("/api/admin/branch-reviewers/branches", { cache: "no-store" });
}

// Reads every grant page; the API caps a page at 100 rows.
export async function listReviewerGrants(): Promise<ReviewerGrant[]> {
  const grants: ReviewerGrant[] = [];
  let cursor: number | null = null;
  do {
    const page: { grants: ReviewerGrant[]; grants_next_cursor: number | null } = await request(
      `/api/admin/branch-reviewers?limit=100${cursor ? `&grant_cursor=${cursor}` : ""}`,
      { cache: "no-store" },
    );
    grants.push(...page.grants);
    cursor = page.grants_next_cursor;
  } while (cursor);
  return grants;
}

export function grantBranchReviewer(
  member: AccountRef,
  branchKey: string,
  reason: string,
): Promise<{ ok: true }> {
  return postJson("/api/admin/branch-reviewers", {
    issuer: member.issuer,
    subject: member.subject,
    branch_keys: [branchKey],
    reason,
  });
}

export function revokeBranchReviewer(grantId: number, reason: string): Promise<{ ok: true }> {
  return postJson(`/api/admin/branch-reviewers/${grantId}/revoke`, { reason });
}
