# `docs/wiki/` — source of truth for the GitHub wiki

The `aukilabs/auki-sdk` wiki at <https://github.com/aukilabs/auki-sdk/wiki> is **not** edited directly. Its content lives here, in `docs/wiki/` on `develop`, and is mirrored to the wiki repo by [`.github/workflows/wiki-mirror.yml`](../../.github/workflows/wiki-mirror.yml) on every push that touches this directory.

## Why this pattern

GitHub wikis are their own git repos (`<repo>.wiki.git`), but they don't support pull-request review natively — anyone with write access can force-push directly. For an actively-developed SDK we want wiki changes to go through the same PR flow as code: discussion, review, CI, history.

So:

- Edits land here via PR against `develop`.
- On merge, CI publishes to `https://github.com/aukilabs/auki-sdk.wiki.git`.
- The wiki repo is treated as a build artifact, not a source.

This file is **not** published — the mirror action excludes `README.md`.

## File naming

GitHub wiki page filenames map to URL slugs. Hyphens stay; spaces become URL-encoded escapes (ugly). Conventions:

- `Home.md` — the wiki's landing page (mandatory name).
- `_Sidebar.md` — sidebar navigation (underscore-prefixed; GitHub-specific).
- `_Footer.md` — optional footer (underscore-prefixed).
- All other pages use hyphenated `Title-Case.md` (e.g., `Quickstart.md`, `Concept-Peer-Owned-Logs.md`). The page title in the wiki UI is the filename with hyphens replaced by spaces.

Page-to-page links use the filename without `.md`: `[Quickstart](Quickstart)`, `[Three IDs](Concept-Three-IDs)`.

## Adding a page

1. Create `docs/wiki/<Hyphenated-Page-Name>.md` on a feature branch.
2. Update `_Sidebar.md` to link it (if user-discoverable).
3. Open a PR. Review like any docs change.
4. Once merged to `develop`, the action publishes it to the wiki within ~1 minute.

## Removing a page

1. `git rm docs/wiki/<page>.md` on a feature branch.
2. Remove its sidebar link.
3. PR + merge.
4. The action's `rsync --delete` removes the page from the wiki on the next sync.

## Local preview

You can clone the published wiki and `git push` directly to test — but don't do that in normal flow. Any direct push will be overwritten by the next mirror sync.

```bash
git clone https://github.com/aukilabs/auki-sdk.wiki.git
```

For pure markdown preview, any local markdown renderer works (`grip`, VS Code preview, `gh repo view`, etc.).

## Bootstrap note

The wiki repo (`auki-sdk.wiki.git`) is auto-created by GitHub the first time someone visits the wiki tab and clicks "Create the first page." If the mirror action fails with "repository not found," that one-time UI bootstrap hasn't happened yet — create a stub page through the wiki UI, then re-run the action.
