// Same-origin API client. The browser only calls the Next.js server, whose
// rewrite proxies `/api/*` to the Rust Axum API. `API_URL` stays server-side.

import type { Role } from "./experience-contract";

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

export type QueryResponse = {
  answer: string;
  insufficient: boolean;
  sources: Source[];
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

export function askQuestion(question: string): Promise<QueryResponse> {
  return request<QueryResponse>("/api/query", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ question }),
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
  linked_person: { link_id: number; person_key: string; username: string | null } | null;
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
  return request<FamilyMembersPage>(`/api/admin/family-members${query ? `?${query}` : ""}`, {
    cache: "no-store",
  });
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
  return postJson("/api/admin/roles", { issuer: member.issuer, subject: member.subject, role, revoke });
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

export function submitDocument(form: FormData): Promise<DocumentSubmissionResult> {
  return request<DocumentSubmissionResult>("/api/documents/submissions", {
    method: "POST",
    body: form,
  });
}

export function listDocumentSubmissions(): Promise<DocumentSubmissionList> {
  return request<DocumentSubmissionList>("/api/documents/submissions");
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

export function processDocument(
  versionId: number,
): Promise<{ ok: true; job_id?: number; processing_status?: string; promoted?: boolean }> {
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
  processed: Array<{ job_id: number; version_id: number; status: string; error: string | null }>;
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
