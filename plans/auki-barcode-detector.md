# Initiative: auki-barcode-detector

Status: active  
Tier: feature  
Stage: 4-system-design  
Skips: _(none)_  
Last updated: 2026-08-07

Living document — update in place when later work changes earlier conclusions.  
**§1 Intent (feature) and §4 System design stay distinct — never merge.**

---

## 1. Intent

- **Customer / internal need:** Apps on the Auki SDK need first-class 1D barcode detection as a Detection Log producer — portable across platforms, independent of any single app’s product/ESL rules.
- **Framing:** Camera sessions already support typed detectors (`DetectorBody`, Detection Logs). Barcode decoding should be a core SDK capability apps configure, not an app-private Vision path.
- **Why now:** Detector runner + registry bodies exist on `develop`; barcode is a common retail/logistics sensor; prior draft over-fit to one consumer’s profiles.
- **Success look like:** `auki-barcode-detector` on `auki-sdk` `develop` registers with `DetectorBody::Barcode`, defaults to all rxing 1D formats, optionally accepts an app-supplied format allowlist using rxing format names, and emits `DetectionFrame` with `type = "barcode"` plus payload + format + corners. Tests cover round-trip + synthetic decode. Docs read as a core SDK detector, not a port of any app or of the QR crate.

## 2. Alternatives

- **Keep decode only in an app (e.g. Expo/Vision)** — blocks non-iOS / other SDK consumers.
- **Hosted vision service** — overkill for portable 1D barcode.
- **Separate detectors monorepo** — not needed; SDK already hosts first-class detector crates.
- **Decision:** in-repo `crates/auki-barcode-detector` as a first-class SDK detector.

## 3. Priority

- **Rank:** Core camera-perception building block.
- **Capacity:** One crate + `DetectorBody::Barcode`; no app migration in v1.
- **Decision:** finish API redesign bounce, then ship PR against `auki-sdk` `develop`.

## 4. System design

- **Products / codebases impacted:** `aukilabs/auki-sdk` only for v1. App cutovers out of scope.
- **Constraints:**
  - Implement `auki_session::CameraDetector` + `RegisteredCameraDetector`.
  - Portable decoder: **`rxing`** (not Apple Vision).
  - No portal/SKU/shelf/regex logic in the detector — app concern.
  - Positioning: first-class SDK detector. Do **not** frame docs/API as a Cactus port, retail-only tool, or “copy of” the QR crate.
- **Interfaces / boundaries:**
  - Crate `auki-barcode-detector`.
  - Output: `DetectionFrame.type = "barcode"`; `data` = JSON `BarcodeDetections { schema_version, codes: [{ payload, format, corners_px }] }`.
  - `format` strings = rxing `BarcodeFormat` names (canonical underscored / enum-style — **pending lock** below).
  - Corners in source-frame pixels, `TL → TR → BR → BL`.
  - **Config:**
    - Default: detect **all 1D** formats rxing supports (exclude 2D: QR, Data Matrix, PDF417, Aztec, MaxiCode, micro-QR variants).
    - Optional: app provides an explicit list of formats to look for (rxing names). Empty/omitted → default 1D set.
  - Remove `BarcodeSymbologyProfile` / Product|Esl|All and any Cactus wire-label collapse.
  - Registry: first-class `DetectorBody::Barcode(Barcode {})` (already landed).
  - Inputs: `raw` luma8/rgb8/YUV_NV12 + `jpeg`.
- **Risks:** Broader default 1D set may increase false positives vs narrow app filters (apps should pass an allowlist when needed); optional list accepting unknown strings needs clear error.
- **Decision:** redesign config + payload format field + docs tone; keep registry body and CameraDetector contracts.

### Decoder backend

- **`rxing`** — `PossibleFormats` hints from config; emit format via rxing naming (no consumer-specific collapse).

### Bounce log (2026-08-07)

- Human: move implementation to real git clone `/Users/robin/Documents/GitHub/auki-sdk` (branch `feat/auki-barcode-detector`); remove cactus `.tmp-auki-sdk` so Cactus work is unblocked.
- ### Bounce log prior (2026-08-07)

- Human: remove symbology profiles; default all rxing 1D; optional format list; rxing format names; scrub Cactus/retail/QR-copy framing → first-class core detector.
- Invalidates prior §4 profiles / Cactus wire labels / §1 framing as Cactus port; S2–S4 docs/tests need revise (S1 registry body still valid).

## 5. Verification design

- **Acceptance:** `cargo test -p auki-barcode-detector` green; default config synthetic EAN-13 → payload + rxing-style `format` + 4 corners; optional allowlist restricts formats; schema round-trip; Detection Log smoke `type == "barcode"`; no Cactus/profile/QR-mirror language in crate README/Cargo.toml crate description.
- **CI:** Workspace member.
- **Out of scope v1:** App Expo cutover, Vision parity, product regex.

## 6. Work breakdown (AI-oriented)

**Done (keep):** S1 registry `Barcode` body.

**Revise / new:**

1. **S5 — Config + format API** — Drop profiles; `BarcodeDetectorConfig { formats: Option<Vec<…>> }` (or equiv); default all 1D; map wire `format` to rxing names; rename payload field `symbology` → `format`; scrub crate docs/comments/Cargo description.
2. **S6 — Tests + docs refresh** — Replace Cactus-collapse tests; assert default EAN-13 `format`; allowlist test; README usage without profiles/Cactus/QR-copy language; touch root/registry/session README wording if it still implies port.

- **Human:** Confirm format-string convention + whether optional list may include 2D; then approve S5–S6; later PR to `aukilabs/auki-sdk`.

## 7. Implementation notes

- Locked: land in `auki-sdk`; backend `rxing`; `DetectorBody::Barcode`; **no** Product/Esl profiles; **no** Cactus collapse.
- Pending human lock: format string canonical form; optional-list 2D policy; payload field rename `symbology`→`format`.
- Prior S2–S4 verification still historically valid against old design; supersede after S5–S6.

## 8. Verification record

### S1 (2026-08-07)
- Review/verify pass — `DetectorBody::Barcode` additive. Still in force.

### S2–S4 (2026-08-07)
- Completed under **superseded** Cactus-profile design. Replaced by S5–S6 after bounce.

## Gate (stage 11)

- **Decision:** _(reopened — bounce)_  
- **Notes:** Do not accept/release until S5–S6 verify.
