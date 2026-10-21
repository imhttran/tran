// Same-origin API client. The browser only calls the Next.js server, whose
// rewrite proxies `/api/*` to the Rust Axum API. `API_URL` stays server-side.

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
