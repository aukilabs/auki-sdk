# Fleet overview, cards and machine inspector

Current Fleet presentation, captured with the actual Web WASM SDK and the local
synthetic DDS/DMS fixture. These images supersede the Fleet images in the older
[control-panel gallery](../control-panel/README.md).

- [Overview and cards — desktop, 1440×960](overview-desktop.png)
- [Machine inspector — desktop, 1440×960](inspector-desktop.png)
- [Overview and cards — mobile, 390×844](overview-mobile.png)
- [Machine details — mobile, 390×844](inspector-mobile.png)
- [Partial work-status observations — desktop, 1280×844](partial-desktop.png)
- [Jobs action names in a fresh session — desktop, 1440×900](jobs-history-desktop.png)

The fixture provides one Domain robot and two compute candidates. Counts are
observations, not capacity or scheduling guarantees. The partial state denies
the busy feed and retains inventory with unknown work status. Mobile overview
shows the restored keyboard focus after leaving machine details with Escape.
Fleet loads on entry and defaults to online machines; the checkbox includes
offline and unknown presence. Overview counts still include the entire snapshot.
The Jobs capture shows action names before either history row has been opened
in the new session.

`npm run test:fleet` regenerates these views under
`test-artifacts/fleet-redesign/` and checks filters, summary scope, desktop/narrow
layouts, initial loading, interrupted-entry recovery, online/offline filtering,
focus restoration, refresh selection, permission failures, Domain
changes, cancellation, logout and retained uncertain Jobs. Capture files are
copied here after visual inspection. The 320×568 and 640×360 layouts are also
checked by the suite. This does not establish live-provider or physical-robot support.

## Validation

Verified on 2026-09-18 with Node 26.3.0:

- `npm run typecheck` — passed.
- `npm test` — 177 passed.
- `node --test tests/fleet-fixture.test.mjs tests/jobs-fixture.test.mjs` — 10 passed.
- `npm run build` — passed, including the locked Web WASM build. Existing unused-import warnings in `auki-sdk-web`, the wasm-pack package-license notice and its update notice remain.
- `npm run test:fleet` — passed with the generated SDK and loopback providers.
- `npm run test:jobs` — passed, including fresh-session action names without data reads or output authorization.
- `npm run test:browser` — passed against production assets with loopback providers.
- `git diff --check`, documentation links and local icon paths — passed.

This change affects the Core Explorer frontend only; SDK APIs, backend contracts
and other bindings are unchanged. Native, Python, Swift and Expo suites, other
browser engines, assistive-technology audits and live-service validation were not run.
