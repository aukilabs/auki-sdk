# Initiative: auki-barcode-detector

Status: integrating  
Tier: feature  
Stage: 10-integrate  
Skips: _(none)_  
Last updated: 2026-08-07

Living document — update in place when later work changes earlier conclusions.  
**§1 Intent (feature) and §4 System design stay distinct — never merge.**

---

## 1. Intent

- **Customer / internal need:** Apps on the Auki SDK need first-class 1D barcode detection as a Detection Log producer — portable across platforms, independent of any single app’s product/ESL rules.
- **Framing:** Camera sessions already support typed detectors (`DetectorBody`, Detection Logs). Barcode decoding should be a core SDK capability apps configure, not an app-private Vision path.
- **Why now:** Detector runner + registry bodies exist on `develop`; barcode is a common camera perception need; prior draft over-fit to one consumer’s profiles.
- **Workspace:** `/Users/robin/Documents/GitHub/auki-sdk` on branch `feat/auki-barcode-detector` (independent of cactus).
- **Harness board:** workspace bucket `/Users/robin/Documents/GitHub/auki-sdk` (not the cactus board). Moved off cactus 2026-08-07.
- **Success look like:** `auki-barcode-detector` on `auki-sdk` `develop` registers with `DetectorBody::Barcode`, defaults to all rxing 1D formats, optionally accepts an app-supplied format allowlist using rxing enum variant names (`EAN_13`, `CODE_128`), and emits `DetectionFrame` with `type = "barcode"` plus payload + format + corners. Tests cover round-trip + synthetic decode. Docs read as a core SDK detector, not a port of any app or of the QR crate.

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
  - **`format` strings (locked 2026-08-07):** rxing `BarcodeFormat` **enum variant names** — e.g. `EAN_13`, `CODE_128`, `RSS_14`. Parse allowlist case-insensitively; emit exact variant spelling.
  - Corners in source-frame pixels, `TL → TR → BR → BL`.
  - **Config:**
    - `BarcodeDetectorConfig { formats: Option<Vec<String>> }` (or typed wrapper).
    - Default (`None` / omit): detect **all 1D** formats rxing supports (CODABAR, CODE_39/93/128, EAN_8/13, ITF, RSS_14, RSS_EXPANDED, TELEPEN, UPC_A, UPC_E, UPC_EAN_EXTENSION, DXFilmEdge as applicable). Exclude 2D by default (QR_CODE, DATA_MATRIX, PDF_417, AZTEC, MAXICODE, MICRO_QR_CODE, RECTANGULAR_MICRO_QR_CODE).
    - Optional explicit list: any rxing format names the app wants (including 2D if requested). Unknown names → config error. Empty `Some(vec![])` → error or treat as default (prefer error: useless config).
  - Remove `BarcodeSymbologyProfile` / Product|Esl|All and any consumer-specific wire-label collapse; payload field `symbology` → `format`.
  - Registry: first-class `DetectorBody::Barcode(Barcode {})` (already on branch).
  - Inputs: `raw` luma8/rgb8/YUV_NV12 + `jpeg`.
- **Risks:** Broader default 1D set may increase false positives (apps pass allowlist when needed); unknown format strings need clear errors.
- **Decision:** redesign config + payload `format` field + docs tone; keep registry body and CameraDetector contracts.

### Decoder backend

- **`rxing`** — `PossibleFormats` hints from config; emit `BarcodeFormat` Debug/variant name (no consumer-specific collapse).

### Bounce log (2026-08-07)

- Human: move implementation to `/Users/robin/Documents/GitHub/auki-sdk` (`feat/auki-barcode-detector`); remove cactus `.tmp-auki-sdk`.
- Human: move harness-board initiative + slices from cactus board → auki-sdk board (2026-08-07).
- Human: remove symbology profiles; default all rxing 1D; optional format list; format names = enum variants (`EAN_13`); scrub Cactus/retail/QR-copy framing.
- Invalidates prior Cactus profiles / wire labels; S2–S4 superseded by S5–S6.

## 5. Verification design

- **Acceptance:** `cargo +1.88.0 test -p auki-barcode-detector` green; default config synthetic EAN-13 → `format == "EAN_13"` + 4 corners; optional allowlist restricts formats; unknown format name rejected; schema round-trip; Detection Log smoke `type == "barcode"`; no Cactus/profile/QR-copy language in crate README / Cargo description.
- **CI:** Workspace member.
- **Out of scope v1:** App Expo cutover, Vision parity, product regex.

## 6. Work breakdown (AI-oriented)

**Done (keep):** S1 registry `Barcode` body (on branch).

**Revise / new:**

1. **S5 — Config + format API** — Drop profiles; optional format list; default all 1D; wire `format` = enum variant; rename `symbology` → `format`; scrub crate docs/comments/Cargo description.
2. **S6 — Tests + docs refresh** — Replace collapse tests; EAN-13 `EAN_13`; allowlist + unknown-format tests; README first-class usage; scrub root/session README “retail/reference port” wording if any.

## 7. Implementation notes

- Locked: `auki-sdk` branch; `rxing`; `DetectorBody::Barcode`; no profiles; format = enum variant (`EAN_13`); default 1D; optional list may include 2D; `symbology`→`format`.
- Approvals: **mass-approve S5–S6** 2026-08-07.

## 8. Verification record

### S1 (2026-08-07)
- Review/verify pass — `DetectorBody::Barcode` additive. Still in force (ported to auki-sdk clone).

### S2–S4 (2026-08-07)
- Superseded by bounce; replaced by S5–S6.

### S5 (2026-08-07)
- Review: approve ([code-reviewer](7cea6b4e-419b-4967-81fa-ecde42b5c198))
- Verify: pass — formats Option; EAN_13; 8 tests; docs scrubbed ([behavior-verifier](1aabacee-7edb-4104-882f-cd1ca490c9ab))
- Code: `crates/auki-barcode-detector/` (no profiles; `format` field; default 1D)

### S6 (2026-08-07)
- Review: approve ([code-reviewer](40552946-5349-4def-8feb-7d8dba082819))
- Verify: pass — `cargo +1.88.0 test -p auki-barcode-detector` 9/9; allowlist + EAN_13 + Detection Log ([behavior-verifier](86e51c64-d4bf-4e82-8162-9658f584da45))
- Code: tests + README polish in `auki-barcode-detector`

## Gate (stage 11)

- **Decision:** _(deferred)_ — human moved board status to **integrating** for further testing before accept (2026-08-07).
- **Notes:** Slices S1/S5–S6 verified; human testing / commit / PR still open on `feat/auki-barcode-detector`.
