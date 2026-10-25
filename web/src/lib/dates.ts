// Date presentation for genealogy facts.
//
// The Rust API stores partial ISO-8601 values (`1877`, `1877-03`,
// `1877-03-15`) plus the precision and the source's own certainty. The browser
// never re-derives a date from prose: it only formats what the server sent, so a
// year-only date can never be rendered as a full calendar day.

export type DatePrecision = "day" | "month" | "year" | "range";
export type DateQualifier = "exact" | "circa" | "before" | "after";

export type StructuredDate = {
  start: string;
  end: string | null;
  precision: DatePrecision;
  qualifier: DateQualifier;
};

const MONTHS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const PRECISIONS: ReadonlySet<string> = new Set(["day", "month", "year", "range"]);
const QUALIFIERS: ReadonlySet<string> = new Set(["exact", "circa", "before", "after"]);

/** True for a partial ISO value the server is allowed to store. */
export function isPartialIsoDate(value: string): boolean {
  return /^\d{4}(-\d{2}(-\d{2})?)?$/.test(value);
}

/** Defensive parse of one structured date. Invalid shapes are dropped. */
export function parseStructuredDate(body: unknown): StructuredDate | null {
  if (!body || typeof body !== "object") return null;
  const data = body as Partial<StructuredDate>;
  if (typeof data.start !== "string" || !isPartialIsoDate(data.start)) return null;
  const end = data.end ?? null;
  if (end !== null && (typeof end !== "string" || !isPartialIsoDate(end))) return null;
  if (typeof data.precision !== "string" || !PRECISIONS.has(data.precision)) return null;
  if (typeof data.qualifier !== "string" || !QUALIFIERS.has(data.qualifier)) return null;
  // The server's constraint guarantees this pairing; the client refuses an
  // inconsistent shape instead of rendering a range with no second bound.
  if ((data.precision === "range") !== (end !== null)) return null;
  return {
    start: data.start,
    end,
    precision: data.precision as DatePrecision,
    qualifier: data.qualifier as DateQualifier,
  };
}

/** One partial ISO value as readable text, preserving its precision. */
export function formatDatePart(value: string): string {
  const [year, month, day] = value.split("-");
  if (!month) return year;
  const monthName = MONTHS[Number(month) - 1] ?? month;
  if (!day) return `${monthName} ${year}`;
  return `${Number(day)} ${monthName} ${year}`;
}

function qualifierPrefix(qualifier: DateQualifier): string {
  switch (qualifier) {
    case "circa":
      return "c. ";
    case "before":
      return "before ";
    case "after":
      return "after ";
    default:
      return "";
  }
}

/** One date as readable text, with its uncertainty kept visible. */
export function formatStructuredDate(date: StructuredDate): string {
  const prefix = qualifierPrefix(date.qualifier);
  if (date.precision === "range" && date.end) {
    return `${prefix}${formatDatePart(date.start)}–${formatDatePart(date.end)}`;
  }
  return `${prefix}${formatDatePart(date.start)}`;
}

/** A short description of the precision, for a detail view. */
export function precisionLabel(precision: DatePrecision): string {
  switch (precision) {
    case "day":
      return "Exact day";
    case "month":
      return "Month and year only";
    case "range":
      return "Stated range";
    default:
      return "Year only";
  }
}

/**
 * A concise lifespan for a tree card.
 *
 * Only a verified birth and death reach this function; the caller passes what the
 * server marked verified. A missing death date is never rendered as living
 * status, and an empty string means "no label".
 */
export function lifespanLabel(
  birth: StructuredDate | null | undefined,
  death: StructuredDate | null | undefined,
): string {
  const birthText = birth ? formatStructuredDate(birth) : "";
  const deathText = death ? formatStructuredDate(death) : "";
  if (birthText && deathText) {
    if (birthText === deathText) return birthText;
    return `${birthText}–${deathText}`;
  }
  if (birthText) return `b. ${birthText}`;
  if (deathText) return `d. ${deathText}`;
  return "";
}

/**
 * The card label, when it is short enough to sit beside the status text.
 *
 * Cards have a fixed width, so a long lifespan (a full-day range, for example)
 * is omitted rather than allowed to collide with the status badge. The detail
 * popup always shows the full value.
 */
export function cardLifespanLabel(
  birth: StructuredDate | null | undefined,
  death: StructuredDate | null | undefined,
  maxLength = 16,
): string {
  const label = lifespanLabel(birth, death);
  return label.length > 0 && label.length <= maxLength ? label : "";
}
