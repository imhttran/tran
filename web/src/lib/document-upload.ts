// Shared document-upload facts for the browser UI.
//
// Keep these in step with the server contract:
//   `DOCUMENT_EXTENSIONS` and `MAX_DOCUMENT_BYTES` in src/document_lifecycle.rs.
// `web/e2e/documents-guidance.spec.ts` fails when these drift apart, so a change
// on either side is caught instead of silently mis-advertising the limit.
// The form advertises exactly what the Rust API accepts; unsupported image or
// word-processing formats are deliberately not offered.

export const DOCUMENT_UPLOAD_ACCEPT = ".md,.markdown,.txt,.pdf";

export const DOCUMENT_UPLOAD_MAX_BYTES = 10 * 1024 * 1024;

/**
 * Human-readable upload limit, for example "10 MiB (10,485,760 bytes)".
 *
 * Grouping is done by hand rather than with `toLocaleString` so the text is
 * identical on the server and in the browser regardless of the host's locale
 * or ICU data.
 */
export function documentUploadLimitLabel(
  bytes: number = DOCUMENT_UPLOAD_MAX_BYTES,
): string {
  const mib = Math.round((bytes / (1024 * 1024)) * 10) / 10;
  const grouped = String(Math.trunc(bytes)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${mib} MiB (${grouped} bytes)`;
}

/**
 * Fictional example used by the first-use guidance. The Copy button copies
 * exactly this text. It is intentionally synthetic and clearly labelled.
 *
 * It opens with a `## ` heading because the Markdown loader only treats that as
 * a section boundary (src/section.rs), so a heading-less `.md` has nothing to
 * index. The same text works unchanged as `.txt`, which is read as one passage.
 */
export const EXAMPLE_FAMILY_STORY = `## Family story

Person: Nguyễn Văn An
Also known as: [Other names, if known]
Relationship: Son of Nguyễn Văn Bình and Trần Thị Lan.
Story: [Describe a memory, experience, or important event.]
Date and place: [Include what you know; approximate dates are fine.]
Source: [Who shared this, or which document supports it.]
Uncertainty: [Explain anything that is not confirmed.]`;
