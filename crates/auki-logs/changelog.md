# Changelog — auki-logs

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 4, 09:24 HKT, 2026

Filesystem layout diagram now lists `tags.jsonl` as a reserved sibling to `manifest.json`, with a one-paragraph note that the auki-logs writer doesn't produce or consume it (TagClaim handling lives outside the crate boundary). Spec gap fix only — the sidecar is fully described in the root [`tags.md`](../../tags.md) but was previously invisible from the per-crate spec, so directory-enumerating tooling could silently miss it. No code changes.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
