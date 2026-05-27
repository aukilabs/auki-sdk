// Domain-name canonicalization + validation for the first-boot prompt.
//
//   • charset:   [a-z0-9-]
//   • length:    1..32 chars after canonicalization
//   • whitespace and capital letters are normalized client-side, not
//     rejected. The user types "Atlanta Warehouse"; the UI shows
//     "atlanta-warehouse" before submit and only the canonical form is
//     persisted.
//
// This module is pure functions on strings so the SDK-side validator
// can mirror it exactly. Keeping the rules here keeps the modal
// trivial — it just renders whatever `canonicalize` returns and
// refuses submit when `validate` errors.
//
// THIS IS PARK-SIDE ONLY UNTIL T1 LANDS. The SDK has the final say on
// what's accepted; Park is currently the only producer of names so
// being strict here is harmless. When T1 ships, this file's
// `validate` is expected to remain a strict subset of (or equal to)
// the SDK's accept set — Park is allowed to be tighter than the SDK,
// never looser.

export const DOMAIN_NAME_MAX = 32;
export const DOMAIN_NAME_MIN = 1;

/** Normalize a user-typed name to the canonical wire form.
 *
 * - Lowercase.
 * - Trim surrounding whitespace.
 * - Replace runs of whitespace with a single hyphen.
 * - Collapse runs of hyphens to a single hyphen.
 * - Strip any character that isn't `[a-z0-9-]` after the above
 *   transforms (covers punctuation, unicode, etc.).
 * - Trim leading / trailing hyphens.
 *
 * The result is always either a valid canonical name (1..32 chars,
 * matches `/^[a-z0-9](-?[a-z0-9])*$/`) or the empty string. The empty
 * string is what the modal renders as "—" in the preview when the
 * user has typed nothing usable. */
export function canonicalize(input: string): string {
  const lowered = input.toLowerCase().trim();
  const dehyphened = lowered
    .replace(/\s+/g, "-")
    .replace(/[^a-z0-9-]+/g, "")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "");
  return dehyphened.slice(0, DOMAIN_NAME_MAX);
}

export type DomainNameValidation =
  | { ok: true; canonical: string }
  | { ok: false; reason: ValidationReason; canonical: string };

export type ValidationReason =
  | "empty"
  | "too_long"
  | "reserved";

/** Reserved names. Empty in v1 per Q12's "no reserved names" lean —
 * exposed as a constant so the SDK side can copy the set verbatim
 * when T1 ships, and so tests can pin the lean. */
export const RESERVED_NAMES: ReadonlySet<string> = new Set();

/** Validate user input. The canonical form is returned in both
 * branches so callers can render the preview without duplicating
 * `canonicalize`. */
export function validate(input: string): DomainNameValidation {
  const canonical = canonicalize(input);
  if (canonical.length < DOMAIN_NAME_MIN) {
    return { ok: false, reason: "empty", canonical };
  }
  if (canonical.length > DOMAIN_NAME_MAX) {
    // Defensive — `canonicalize` already truncates, so this branch is
    // unreachable as long as `slice(0, MAX)` is in canonicalize. Kept
    // explicit so a future relaxation of canonicalize surfaces here.
    return { ok: false, reason: "too_long", canonical };
  }
  if (RESERVED_NAMES.has(canonical)) {
    return { ok: false, reason: "reserved", canonical };
  }
  return { ok: true, canonical };
}

export function reasonLabel(reason: ValidationReason): string {
  switch (reason) {
    case "empty":
      return "Enter a name.";
    case "too_long":
      return `Too long — max ${DOMAIN_NAME_MAX} characters after normalization.`;
    case "reserved":
      return "That name is reserved.";
  }
}
