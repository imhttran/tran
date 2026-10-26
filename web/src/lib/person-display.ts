// Shared display helpers for a person. These are presentation-only: they never
// infer a fact and never change the stored person label.

/**
 * The first letter of the display name, for an initials avatar. Whitespace is
 * trimmed first, so a padded name still yields its first real letter. Names
 * written with Vietnamese diacritics keep their first letter as written. A name
 * with no usable letter (empty, or only punctuation) falls back to a stable
 * placeholder so the card never renders an empty or broken glyph.
 */
export function initialLetter(name: string): string {
  const trimmed = name.trim();
  const first = Array.from(trimmed)[0] ?? "";
  return /^\p{L}$/u.test(first) ? first.toUpperCase() : "?";
}
