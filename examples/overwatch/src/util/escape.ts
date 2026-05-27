// HTML-entity escaper. Used wherever we hand a user-controlled string
// into innerHTML / template strings — sensor IDs, peer ids, daemon
// names, file paths, all of which can contain `<`, `>`, `&`, quotes.
// One canonical implementation; do not re-define this locally.

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    c === "&" ? "&amp;" :
    c === "<" ? "&lt;" :
    c === ">" ? "&gt;" :
    c === '"' ? "&quot;" : "&#39;"
  );
}
