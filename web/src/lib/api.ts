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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, init);
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
