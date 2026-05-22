# Auki Proto Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a committed generated Rust `auki-proto` crate, add ignored local generation paths for JavaScript/TypeScript, Swift, and Python bindings, then deprecate `auki-datatypes` as a compatibility shim.

**Architecture:** Move canonical `.proto` schemas to root `proto/auki/`. Generate and commit only `crates/auki-proto`. Generate JavaScript/TypeScript, Swift, and Python outputs under `bindings/` for local consumers and smoke checks, but keep those generated directories ignored by git. Keep transport and lifecycle behavior in `auki-network` and `auki-domain`.

**Tech Stack:** Rust 2024, prost/prost-build for Rust generation, repo `just` recipes, protoc, platform-native protobuf generators, existing locked wire-vector tests.

---

## File Structure

- Create: `proto/README.md` - schema ownership and generation contract.
- Create: `proto/auki/*.proto` - canonical schema source moved from `crates/auki-datatypes/proto/`.
- Create: `tools/proto-gen-rust/Cargo.toml` - Rust codegen utility using `prost-build`.
- Create: `tools/proto-gen-rust/src/main.rs` - generates checked-in prost files.
- Create: `scripts/generate-rust-proto.sh` - root script for Rust protobuf generation.
- Create: `scripts/generate-proto.sh` - root script that runs Rust generation plus optional local non-Rust binding generation.
- Modify: `justfile` - add `generate-rust-proto` and `generate-proto` recipes.
- Create: `crates/auki-proto/README.md` - Rust generated proto package spec.
- Create: `crates/auki-proto/Cargo.toml` - Rust package manifest.
- Create: `crates/auki-proto/src/lib.rs` - generated module includes and Rust-only helper shell.
- Create: `crates/auki-proto/src/logs.rs` - optional `auki_logs::LogPayload` impls.
- Create: `crates/auki-proto/src/generated/*.rs` - checked-in generated prost output.
- Modify: `crates/auki-datatypes/*` - convert to deprecated Rust shim over `auki-proto`.
- Modify: Rust consumers currently importing `auki_datatypes::*` to import `auki_proto::*`.
- Do not commit: `bindings/javascript/auki-proto/*` - generated local browser protobuf output ignored by git.
- Do not commit: `bindings/swift/auki-proto/*` - generated local SwiftProtobuf output ignored by git.
- Do not commit: `bindings/python/auki-proto/*` - generated local Python protobuf output ignored by git.
- Modify: changelogs at leaf, parent, and root levels immediately after each change.

## Task 1: Move Schema Source To Root

**Files:**
- Create: `proto/README.md`
- Create: `proto/auki/audio.proto`
- Create: `proto/auki/audio_stream.proto`
- Create: `proto/auki/camera.proto`
- Create: `proto/auki/detection.proto`
- Create: `proto/auki/joint_encoders.proto`
- Create: `proto/auki/joint_encoders_stream.proto`
- Create: `proto/auki/point_cloud.proto`
- Create: `proto/auki/point_cloud_stream.proto`
- Create: `proto/auki/pose.proto`
- Create: `proto/auki/stream.proto`
- Create: `proto/auki/time_transform.proto`
- Modify: `changelog.md`

- [ ] **Step 1: Copy existing schemas**

Run:

```bash
mkdir -p proto/auki
cp crates/auki-datatypes/proto/*.proto proto/auki/
```

Expected: `find proto/auki -maxdepth 1 -name '*.proto' | wc -l` prints `11`.

- [ ] **Step 2: Verify byte-identical copy**

Run:

```bash
for file in crates/auki-datatypes/proto/*.proto; do
  base="$(basename "$file")"
  diff -u "$file" "proto/auki/$base"
done
```

Expected: no output.

- [ ] **Step 3: Add schema ownership README**

Create `proto/README.md`:

```markdown
# Auki Protobuf Schemas

This directory is the canonical source for Auki protobuf schemas.

Platform packages are generated from `proto/auki/*.proto`:

- Rust: `crates/auki-proto`
- JavaScript/TypeScript: `bindings/javascript/auki-proto` local generated output, ignored by git
- Swift/iOS: `bindings/swift/auki-proto` local generated output, ignored by git
- Python: `bindings/python/auki-proto` local generated output, ignored by git

Do not edit generated protobuf bindings by hand. Edit the `.proto` files here,
then run `just generate-proto`. Commit Rust output under `crates/auki-proto`;
do not commit generated non-Rust binding output under `bindings/`.
```

- [ ] **Step 4: Record root changelog**

Prepend to `changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

**Canonical protobuf schemas moved to `proto/auki/`.** The repo now has a platform-neutral schema source for the forthcoming `auki-proto` generated package family.
```

- [ ] **Step 5: Commit**

Run:

```bash
git add proto changelog.md
git commit -m "chore: move protobuf schemas to root"
```

## Task 2: Add Rust `auki-proto` Generator And Package Shell

**Files:**
- Create: `tools/proto-gen-rust/Cargo.toml`
- Create: `tools/proto-gen-rust/src/main.rs`
- Create: `scripts/generate-rust-proto.sh`
- Modify: `justfile`
- Modify: `Cargo.toml`
- Create: `crates/auki-proto/Cargo.toml`
- Create: `crates/auki-proto/README.md`
- Create: `crates/auki-proto/src/lib.rs`
- Create: `crates/auki-proto/src/logs.rs`
- Create: `crates/auki-proto/changelog.md`
- Create: `crates/auki-proto/parking_lot.md`
- Create: `crates/auki-proto/src/README.md`

- [ ] **Step 1: Add Rust codegen utility manifest**

Create `tools/proto-gen-rust/Cargo.toml`:

```toml
[package]
name = "auki-proto-gen-rust"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
prost-build = "0.13"
protoc-bin-vendored = "3"
```

- [ ] **Step 2: Add Rust codegen utility**

Create `tools/proto-gen-rust/src/main.rs`:

```rust
use std::fs;
use std::path::{Path, PathBuf};

const PROTOS: &[&str] = &[
    "audio.proto",
    "audio_stream.proto",
    "camera.proto",
    "detection.proto",
    "joint_encoders.proto",
    "joint_encoders_stream.proto",
    "point_cloud.proto",
    "point_cloud_stream.proto",
    "pose.proto",
    "stream.proto",
    "time_transform.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::current_dir()?;
    let proto_root = repo.join("proto/auki");
    let out_dir = repo.join("crates/auki-proto/src/generated");

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;

    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("vendored protoc binary not available for this platform");
    let proto_paths: Vec<PathBuf> = PROTOS.iter().map(|name| proto_root.join(name)).collect();

    prost_build::Config::new()
        .out_dir(&out_dir)
        .protoc_executable(protoc)
        .compile_protos(&proto_paths, &[proto_root])?;

    assert_generated(&out_dir, "auki.camera.rs")?;
    assert_generated(&out_dir, "auki.stream.rs")?;
    Ok(())
}

fn assert_generated(out_dir: &Path, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = out_dir.join(file);
    if !path.exists() {
        return Err(format!("expected generated file {}", path.display()).into());
    }
    Ok(())
}
```

- [ ] **Step 3: Add root Rust generation script**

Create `scripts/generate-rust-proto.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo run --manifest-path tools/proto-gen-rust/Cargo.toml
cargo fmt -p auki-proto
```

Run:

```bash
chmod +x scripts/generate-rust-proto.sh
```

- [ ] **Step 4: Add `just` recipes**

Modify `justfile`:

```just
generate-rust-proto:
    bash scripts/generate-rust-proto.sh

generate-proto:
    bash scripts/generate-proto.sh
```

Create `scripts/generate-proto.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
just generate-rust-proto
```

Run:

```bash
chmod +x scripts/generate-proto.sh
```

Expected: `just generate-proto` initially runs only Rust generation. Later tasks extend this script with non-Rust local generation, but only Rust generated output is committed.

- [ ] **Step 5: Add `auki-proto` workspace member**

Modify root `Cargo.toml` members list to include:

```toml
    "crates/auki-proto",
```

- [ ] **Step 6: Add `auki-proto` manifest**

Create `crates/auki-proto/Cargo.toml`:

```toml
[package]
name = "auki-proto"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Generated Rust protobuf bindings for Auki schemas."

[features]
default = []
logs = ["dep:auki-logs"]

[dependencies]
prost = "0.13"
auki-logs = { path = "../auki-logs", optional = true }

[dev-dependencies]
auki-hash = { path = "../auki-hash" }
serde_json = "1"
tempfile = "3"
```

- [ ] **Step 7: Add Rust package shell**

Create `crates/auki-proto/src/lib.rs`:

```rust
//! Generated Rust protobuf bindings for Auki schemas.
//!
//! The canonical schema source lives under `proto/auki/`. Regenerate this crate
//! with `just generate-rust-proto`.

#![allow(missing_docs, clippy::derive_partial_eq_without_eq)]

#[cfg(feature = "logs")]
mod logs;

pub mod audio {
    include!("generated/auki.audio.rs");
}

pub mod audio_stream {
    include!("generated/auki.audio_stream.rs");
}

pub mod camera {
    include!("generated/auki.camera.rs");
}

pub mod detection {
    include!("generated/auki.detection.rs");
}

pub mod joint_encoders {
    include!("generated/auki.joint_encoders.rs");
}

pub mod joint_encoders_stream {
    include!("generated/auki.joint_encoders_stream.rs");
}

pub mod point_cloud {
    include!("generated/auki.point_cloud.rs");
}

pub mod point_cloud_stream {
    include!("generated/auki.point_cloud_stream.rs");
}

pub mod pose {
    include!("generated/auki.pose.rs");
}

pub mod stream {
    include!("generated/auki.stream.rs");

    impl StreamMessage {
        pub fn request(req: StreamRequest) -> Self {
            Self { variant: Some(stream_message::Variant::Request(req)) }
        }

        pub fn accept(descriptor: StreamDescriptor) -> Self {
            Self { variant: Some(stream_message::Variant::Accept(descriptor)) }
        }

        pub fn decline(reason: DeclineReason) -> Self {
            Self { variant: Some(stream_message::Variant::Decline(reason)) }
        }

        pub fn entry(entry: StreamEntry) -> Self {
            Self { variant: Some(stream_message::Variant::Entry(entry)) }
        }

        pub fn end_of_stream(reason: EndReason) -> Self {
            Self { variant: Some(stream_message::Variant::EndOfStream(reason)) }
        }
    }

    impl DeclineReason {
        pub fn sensor_not_found() -> Self {
            Self {
                kind: Some(decline_reason::Kind::SensorNotFound(
                    decline_reason::SensorNotFound {},
                )),
            }
        }

        pub fn sensor_unavailable() -> Self {
            Self {
                kind: Some(decline_reason::Kind::SensorUnavailable(
                    decline_reason::SensorUnavailable {},
                )),
            }
        }

        pub fn producer_shutting_down() -> Self {
            Self {
                kind: Some(decline_reason::Kind::ProducerShuttingDown(
                    decline_reason::ProducerShuttingDown {},
                )),
            }
        }

        pub fn other(detail: impl Into<String>) -> Self {
            Self {
                kind: Some(decline_reason::Kind::Other(decline_reason::Other {
                    detail: detail.into(),
                })),
            }
        }
    }

    impl EndReason {
        pub fn source_ended() -> Self {
            Self { kind: Some(end_reason::Kind::SourceEnded(end_reason::SourceEnded {})) }
        }

        pub fn producer_shutting_down() -> Self {
            Self {
                kind: Some(end_reason::Kind::ProducerShuttingDown(
                    end_reason::ProducerShuttingDown {},
                )),
            }
        }

        pub fn session_ended() -> Self {
            Self { kind: Some(end_reason::Kind::SessionEnded(end_reason::SessionEnded {})) }
        }

        pub fn producer_error(detail: impl Into<String>) -> Self {
            Self {
                kind: Some(end_reason::Kind::ProducerError(end_reason::ProducerError {
                    detail: detail.into(),
                })),
            }
        }
    }
}

pub mod time_transform {
    include!("generated/auki.time_transform.rs");
}
```

- [ ] **Step 8: Add optional log payload impls**

Create `crates/auki-proto/src/logs.rs`:

```rust
macro_rules! impl_log_payload {
    ($t:ty) => {
        impl ::auki_logs::LogPayload for $t {
            fn encode(&self) -> ::std::vec::Vec<u8> {
                ::prost::Message::encode_to_vec(self)
            }

            fn decode(bytes: &[u8]) -> ::std::result::Result<Self, ::std::string::String> {
                <Self as ::prost::Message>::decode(bytes).map_err(|e| e.to_string())
            }
        }
    };
}

impl_log_payload!(crate::audio::AudioLogEntry);
impl_log_payload!(crate::camera::CameraFrame);
impl_log_payload!(crate::detection::DetectionFrame);
impl_log_payload!(crate::joint_encoders::JointEncodersLogEntry);
impl_log_payload!(crate::point_cloud::PointCloudLogEntry);
impl_log_payload!(crate::pose::SpatialTransform);
impl_log_payload!(crate::time_transform::TimeTransformEntry);
```

- [ ] **Step 9: Add package docs**

Create `crates/auki-proto/README.md`:

```markdown
# auki-proto

Generated Rust protobuf bindings for Auki schemas.

Canonical schemas live under `../../proto/auki/`. Run `just generate-rust-proto`
after editing a schema. Generated Rust files are checked in under
`src/generated/` so downstream crates do not run protobuf codegen during normal
Cargo builds.

This crate owns data types and codecs only. Transport handlers live in
`auki-network`; app lifecycle lives in `auki-domain`.
```

Create `crates/auki-proto/src/README.md`:

```markdown
# auki-proto/src

Implementation shell for generated Rust protobuf bindings.

- `lib.rs` includes generated prost modules and Rust-only message constructors.
- `logs.rs` implements `auki_logs::LogPayload` behind the `logs` feature.
- `generated/` is produced by `just generate-rust-proto`.
```

Create `crates/auki-proto/parking_lot.md`:

```markdown
# Parking lot — auki-proto

Open questions for generated Rust protobuf bindings.

---

No open questions.
```

Create `crates/auki-proto/changelog.md`:

```markdown
# Changelog — auki-proto

Append-only timeline of changes for the generated Rust protobuf package. Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

Created the `auki-proto` Rust package shell for generated prost bindings from root `proto/auki` schemas. The package keeps generated code checked in and limits hand-written Rust to module includes, message constructors, and optional `auki-logs` payload impls.
```

- [ ] **Step 10: Generate Rust protobuf output**

Run:

```bash
just generate-rust-proto
```

Expected: `crates/auki-proto/src/generated/auki.camera.rs` and `crates/auki-proto/src/generated/auki.stream.rs` exist.

- [ ] **Step 11: Verify package compiles**

Run:

```bash
cargo check -p auki-proto
cargo check -p auki-proto --features logs
```

Expected: both commands pass.

- [ ] **Step 12: Commit**

Run:

```bash
git add Cargo.toml justfile scripts/generate-rust-proto.sh scripts/generate-proto.sh tools/proto-gen-rust crates/auki-proto
git commit -m "feat: add generated rust auki-proto crate"
```

## Task 3: Move Locked Rust Wire Vectors To `auki-proto`

**Files:**
- Modify: `crates/auki-proto/src/lib.rs`
- Modify: `crates/auki-proto/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Copy locked-vector tests**

Copy the existing `#[cfg(test)] mod tests` block from `crates/auki-datatypes/src/lib.rs` into `crates/auki-proto/src/lib.rs`.

Keep the test bodies byte-for-byte unless an import path names `auki_datatypes`; change those imports to `auki_proto` or `crate`.

- [ ] **Step 2: Run the new tests**

Run:

```bash
cargo test -p auki-proto
```

Expected: every locked vector that currently passes under `auki-datatypes` also passes under `auki-proto`.

- [ ] **Step 3: Add changelog entries**

Prepend to `crates/auki-proto/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

Moved the Rust locked protobuf wire-vector tests into `auki-proto`, making the generated Rust package the byte-equivalence authority for root `proto/auki` schemas.
```

Prepend one-line summaries to `crates/changelog.md` and root `changelog.md` following the existing format.

- [ ] **Step 4: Commit**

Run:

```bash
git add crates/auki-proto crates/changelog.md changelog.md
git commit -m "test: move protobuf locked vectors to auki-proto"
```

## Task 4: Convert `auki-datatypes` To A Deprecated Shim

**Files:**
- Modify: `crates/auki-datatypes/Cargo.toml`
- Delete: `crates/auki-datatypes/build.rs`
- Delete: `crates/auki-datatypes/proto/*.proto`
- Modify: `crates/auki-datatypes/src/lib.rs`
- Modify: `crates/auki-datatypes/README.md`
- Modify: `crates/auki-datatypes/src/readme.md`
- Modify: `crates/auki-datatypes/src/sprint.md`
- Modify: `crates/auki-datatypes/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Replace `auki-datatypes` dependencies**

Change `crates/auki-datatypes/Cargo.toml` to:

```toml
[package]
name = "auki-datatypes"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Deprecated compatibility shim re-exporting auki-proto."

[dependencies]
auki-proto = { path = "../auki-proto", features = ["logs"] }
```

- [ ] **Step 2: Replace library source**

Replace `crates/auki-datatypes/src/lib.rs` with:

```rust
//! Deprecated compatibility shim for the former `auki-datatypes` crate.
//!
//! New code should depend on `auki-proto` and import `auki_proto::*`.

pub use auki_proto::*;
```

- [ ] **Step 3: Remove old build-time codegen files**

Run:

```bash
rm crates/auki-datatypes/build.rs
rm -r crates/auki-datatypes/proto
```

Expected: no `.proto` source remains under `crates/auki-datatypes`.

- [ ] **Step 4: Update datatypes docs**

Replace the opening paragraph of `crates/auki-datatypes/README.md` with:

```markdown
# auki-datatypes

Deprecated Rust compatibility shim for [`auki-proto`](../auki-proto).

The protobuf schemas moved to [`../../proto/auki`](../../proto/auki), and Rust
generated protobuf bindings now live in [`auki-proto`](../auki-proto). New code
should depend on `auki-proto`. This crate temporarily re-exports `auki-proto`
so older workspace and downstream imports can migrate without a one-commit flag
day.
```

Update `crates/auki-datatypes/src/readme.md` and `src/sprint.md` to state that active work has moved to `auki-proto`.

- [ ] **Step 5: Verify shim**

Run:

```bash
cargo check -p auki-datatypes
cargo test -p auki-datatypes
```

Expected: both commands pass. `cargo test -p auki-datatypes` may run zero tests after locked-vector tests move to `auki-proto`.

- [ ] **Step 6: Add changelog entries**

Prepend to `crates/auki-datatypes/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

Deprecated `auki-datatypes` in favor of `auki-proto`. The crate is now a compatibility shim re-exporting generated Rust protobuf types from `auki-proto`; canonical schemas live under root `proto/auki`.
```

Prepend one-line summaries to `crates/changelog.md` and root `changelog.md`.

- [ ] **Step 7: Commit**

Run:

```bash
git add crates/auki-datatypes crates/changelog.md changelog.md
git commit -m "chore: deprecate auki-datatypes shim"
```

## Task 5: Port Rust Workspace Consumers To `auki-proto`

**Files:**
- Modify: `crates/auki-geometry/Cargo.toml`
- Modify: `crates/auki-geometry/src/lib.rs`
- Modify: `crates/auki-time/Cargo.toml`
- Modify: `crates/auki-time/src/lib.rs`
- Modify: `crates/auki-network/Cargo.toml`
- Modify: `crates/auki-network/src/stream_protocol.rs`
- Modify: `crates/auki-network/src/stream_runtime.rs`
- Modify: `crates/auki-ros-adapter/Cargo.toml`
- Modify: `crates/auki-ros-adapter/src/lib.rs`
- Modify: `bindings/python/auki-network-py/Cargo.toml`
- Modify: `bindings/python/auki-network-py/src/stream_types.rs`
- Modify: docs and changelogs that name `auki-datatypes` as the active protobuf crate

- [ ] **Step 1: Find direct Rust imports**

Run:

```bash
rg -n "auki[-_]datatypes|auki_datatypes" Cargo.toml crates bindings examples -g 'Cargo.toml' -g '*.rs'
```

Expected: output lists all direct code imports and dependency declarations.

- [ ] **Step 2: Update Cargo dependency names**

Apply these dependency replacements:

```toml
# Before
auki-datatypes = { path = "../auki-datatypes" }

# After, for crates that only need generated prost messages
auki-proto = { path = "../auki-proto" }

# After, for crates that write/read generated messages through auki-logs::Log<T>
auki-proto = { path = "../auki-proto", features = ["logs"] }
```

Use `features = ["logs"]` in `auki-time` because `Sampler` writes `Log<TimeTransformEntry>`.

- [ ] **Step 3: Update Rust imports**

Replace import paths:

```rust
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
```

with:

```rust
use auki_proto::pose::{Quat, SpatialTransform, Vec3};
```

Repeat for `audio`, `audio_stream`, `camera`, `detection`, `joint_encoders`, `joint_encoders_stream`, `point_cloud`, `point_cloud_stream`, `stream`, and `time_transform`.

- [ ] **Step 4: Update `auki-network` feature list**

In `crates/auki-network/Cargo.toml`, change the `swarm` feature dependency from:

```toml
"dep:auki-datatypes"
```

to:

```toml
"dep:auki-proto"
```

and change the dependency block from:

```toml
auki-datatypes = { path = "../auki-datatypes", optional = true }
```

to:

```toml
auki-proto = { path = "../auki-proto", optional = true }
```

- [ ] **Step 5: Verify workspace code no longer imports `auki_datatypes`**

Run:

```bash
rg -n "auki_datatypes|auki-datatypes" crates bindings examples -g 'Cargo.toml' -g '*.rs'
```

Expected: no Rust source imports remain. Documentation references may remain until Task 7 updates docs.

- [ ] **Step 6: Run targeted checks**

Run:

```bash
cargo check -p auki-geometry
cargo check -p auki-time
cargo check -p auki-network --features swarm
cargo check -p auki-ros-adapter
cargo check -p auki-network-py
```

Expected: all commands pass.

- [ ] **Step 7: Add changelog entries**

Prepend leaf changelog entries for every crate whose Rust dependency changed. Prepend parent summaries to `crates/changelog.md`, `bindings/python/changelog.md` if Python binding Rust code changed, `bindings/changelog.md`, and root `changelog.md`.

- [ ] **Step 8: Commit**

Run:

```bash
git add crates bindings Cargo.toml Cargo.lock changelog.md
git commit -m "refactor: port rust consumers to auki-proto"
```

## Task 6: Add Ignored Non-Rust Protobuf Generation

**Files:**
- Create: `scripts/generate-javascript-proto.sh`
- Create: `scripts/generate-swift-proto.sh`
- Create: `scripts/generate-python-proto.sh`
- Modify: `scripts/generate-proto.sh`
- Modify: `justfile`
- Modify: `bindings/README.md`
- Modify: `bindings/javascript/README.md`
- Modify: `bindings/swift/README.md`
- Modify: `bindings/python/README.md`
- Modify: `bindings/changelog.md`
- Modify: `changelog.md`
- Generate locally, do not commit: `bindings/javascript/auki-proto/`
- Generate locally, do not commit: `bindings/swift/auki-proto/`
- Generate locally, do not commit: `bindings/python/auki-proto/`

- [ ] **Step 1: Confirm non-Rust generated output is ignored**

Run:

```bash
rg -n "^/bindings/$" .gitignore
```

Expected:

```text
.gitignore:8:/bindings/
```

This broad ignore keeps new generated binding directories untracked while preserving tracked files that already live under `bindings/`.

- [ ] **Step 2: Add JavaScript/TypeScript generation script**

Create `scripts/generate-javascript-proto.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/javascript/auki-proto"
src="$pkg/src"

rm -rf "$pkg"
mkdir -p "$src"

cat > "$pkg/package.json" <<'JSON'
{
  "name": "@aukilabs/auki-proto",
  "type": "module",
  "version": "0.0.0",
  "license": "MIT",
  "description": "Generated JavaScript/TypeScript protobuf bindings for Auki schemas.",
  "private": true,
  "dependencies": {
    "@bufbuild/protobuf": "^2.0.0"
  },
  "devDependencies": {
    "@bufbuild/protoc-gen-es": "^2.0.0"
  }
}
JSON

cat > "$pkg/README.md" <<'MD'
# @aukilabs/auki-proto

Generated JavaScript/TypeScript protobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-javascript-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

(
  cd "$pkg"
  npm install
)

PATH="$PWD/$pkg/node_modules/.bin:$PATH" \
protoc \
  -I proto \
  --es_out "$src" \
  --es_opt target=ts,import_extension=js \
  proto/auki/*.proto
```

Run:

```bash
chmod +x scripts/generate-javascript-proto.sh
```

- [ ] **Step 3: Add SwiftProtobuf generation script**

Create `scripts/generate-swift-proto.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/swift/auki-proto"
src="$pkg/Sources/AukiProto"

rm -rf "$pkg"
mkdir -p "$src"

cat > "$pkg/Package.swift" <<'SWIFT'
// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "auki-proto",
    platforms: [.iOS(.v13), .macOS(.v12)],
    products: [
        .library(name: "AukiProto", targets: ["AukiProto"])
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-protobuf.git", from: "1.38.0")
    ],
    targets: [
        .target(
            name: "AukiProto",
            dependencies: [.product(name: "SwiftProtobuf", package: "swift-protobuf")]
        )
    ]
)
SWIFT

cat > "$pkg/README.md" <<'MD'
# AukiProto

Generated SwiftProtobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-swift-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

command -v protoc-gen-swift >/dev/null 2>&1 || {
  echo "protoc-gen-swift is required. Install SwiftProtobuf's protoc plugin." >&2
  exit 1
}

protoc \
  -I proto \
  --swift_out "$src" \
  proto/auki/*.proto
```

Run:

```bash
chmod +x scripts/generate-swift-proto.sh
```

- [ ] **Step 4: Add Python protobuf generation script**

Create `scripts/generate-python-proto.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/python/auki-proto"

rm -rf "$pkg"
mkdir -p "$pkg"

cat > "$pkg/pyproject.toml" <<'TOML'
[project]
name = "auki-proto"
version = "0.0.0"
description = "Generated Python protobuf bindings for Auki schemas."
requires-python = ">=3.10"
dependencies = ["protobuf>=5"]
TOML

cat > "$pkg/README.md" <<'MD'
# auki-proto

Generated Python protobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-python-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

protoc \
  -I proto \
  --python_out "$pkg" \
  proto/auki/*.proto

find "$pkg" -type d -exec sh -c 'touch "$0/__init__.py"' {} \;
```

Run:

```bash
chmod +x scripts/generate-python-proto.sh
```

- [ ] **Step 5: Wire aggregate generator and just recipes**

Modify `scripts/generate-proto.sh` so it runs committed Rust generation first and ignored non-Rust generation after:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

just generate-rust-proto
bash scripts/generate-javascript-proto.sh
bash scripts/generate-swift-proto.sh
bash scripts/generate-python-proto.sh
```

Add or update these recipes in `justfile`:

```make
generate-proto:
    bash scripts/generate-proto.sh

generate-javascript-proto:
    bash scripts/generate-javascript-proto.sh

generate-swift-proto:
    bash scripts/generate-swift-proto.sh

generate-python-proto:
    bash scripts/generate-python-proto.sh
```

Keep the existing `generate-rust-proto` recipe from Task 2 unchanged.

- [ ] **Step 6: Generate all platform outputs locally**

Run:

```bash
just generate-proto
```

Expected:

```text
bindings/javascript/auki-proto/src
bindings/swift/auki-proto/Sources/AukiProto
bindings/python/auki-proto
```

contain generated files. The command requires Node/npm, `protoc`, SwiftProtobuf's `protoc-gen-swift`, and the Python generator built into `protoc`.

- [ ] **Step 7: Verify non-Rust output is ignored and unstaged**

Run:

```bash
git status --short bindings/javascript/auki-proto bindings/swift/auki-proto bindings/python/auki-proto
```

Expected: no output.

Run:

```bash
git status --ignored --short bindings/javascript/auki-proto bindings/swift/auki-proto bindings/python/auki-proto
```

Expected:

```text
!! bindings/javascript/auki-proto/
!! bindings/swift/auki-proto/
!! bindings/python/auki-proto/
```

- [ ] **Step 8: Document local non-Rust generation**

In `bindings/README.md`, add:

```markdown
Generated protobuf outputs for JavaScript/TypeScript, Swift, and Python are local binding artifacts. Run `just generate-proto` to refresh them under:

- `bindings/javascript/auki-proto/`
- `bindings/swift/auki-proto/`
- `bindings/python/auki-proto/`

These generated directories are ignored by git. The only generated protobuf output committed by this repo is the Rust crate at `crates/auki-proto`.
```

In each language README, add the language-specific command:

```markdown
Run `just generate-javascript-proto` to create the local ignored `bindings/javascript/auki-proto/` output.
```

```markdown
Run `just generate-swift-proto` to create the local ignored `bindings/swift/auki-proto/` output.
```

```markdown
Run `just generate-python-proto` to create the local ignored `bindings/python/auki-proto/` output.
```

- [ ] **Step 9: Add changelog entries**

Prepend to `bindings/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

Added generator scripts for local ignored JavaScript/TypeScript, Swift, and Python protobuf outputs under `bindings/`; the generated non-Rust `auki-proto` artifacts remain uncommitted.
```

Propagate to root `changelog.md`.

- [ ] **Step 10: Commit only scripts and tracked docs**

Run:

```bash
git add scripts/generate-javascript-proto.sh scripts/generate-swift-proto.sh scripts/generate-python-proto.sh scripts/generate-proto.sh justfile bindings/README.md bindings/javascript/README.md bindings/swift/README.md bindings/python/README.md bindings/changelog.md changelog.md
git status --short bindings/javascript/auki-proto bindings/swift/auki-proto bindings/python/auki-proto
git commit -m "feat: add local non-rust auki-proto generation"
```

Expected before commit: the `git status --short` command for the generated non-Rust directories prints no output.

## Task 7: Documentation Cleanup And Deprecation Sweep

**Files:**
- Modify: `README.md`
- Modify: `crates/README.md`
- Modify: `bindings/README.md`
- Modify: `bindings/python/README.md`
- Modify: `bindings/javascript/README.md`
- Modify: `crates/auki-logs/README.md`
- Modify: `crates/auki-manifests/README.md`
- Modify: `crates/auki-registry/README.md`
- Modify: `crates/auki-network/README.md`
- Modify: `crates/auki-time/README.md`
- Modify: `crates/auki-geometry/README.md`
- Modify: all touched changelogs

- [ ] **Step 1: Replace active-doc terminology**

Run:

```bash
rg -n "auki-datatypes|auki_datatypes|auki-datatypes-py|auki_datatypes" README.md crates bindings docs -g '*.md'
```

For active docs, replace the active protobuf crate/package name with `auki-proto`. Preserve historical changelog entries verbatim.

- [ ] **Step 2: Update root crate table**

In `README.md` and `crates/README.md`, add an `auki-proto` row:

```markdown
| [`auki-proto`](crates/auki-proto) | Generated Rust protobuf bindings from root `proto/auki` schemas. Rust member of the cross-platform `auki-proto` package family. |
```

Change the `auki-datatypes` row to:

```markdown
| [`auki-datatypes`](crates/auki-datatypes) | Deprecated compatibility shim re-exporting `auki-proto`; new code should use `auki-proto`. |
```

- [ ] **Step 3: Update binding README tables**

In `bindings/README.md`, list `auki-proto` as:

```markdown
| `auki-proto` | Generated protobuf outputs. Rust is committed as `crates/auki-proto`; JavaScript/TypeScript, Swift, and Python are generated locally under ignored `bindings/*/auki-proto/` directories. |
```

In `bindings/python/README.md`, do not add a version-controlled Python `auki-proto` package row. If `auki-datatypes-py` is still documented, mark it as deprecated compatibility state and add:

```markdown
The future Python `auki-proto` output is generated from root `proto/auki` schemas by `just generate-python-proto` into ignored `bindings/python/auki-proto/`. It is not committed by this migration.
```

In `bindings/javascript/README.md`, add:

```markdown
The browser TypeScript protobuf output is generated from root `proto/auki` schemas by `just generate-javascript-proto` into ignored `bindings/javascript/auki-proto/`. It is not committed by this migration.
```

In `bindings/swift/README.md`, add:

```markdown
The Swift protobuf output is generated from root `proto/auki` schemas by `just generate-swift-proto` into ignored `bindings/swift/auki-proto/`. It is not committed by this migration.
```

- [ ] **Step 4: Run doc search again**

Run:

```bash
rg -n "auki-datatypes|auki_datatypes|auki-datatypes-py|auki_datatypes" README.md crates bindings docs -g '*.md'
```

Expected: remaining matches are historical changelog entries, deprecation notes, or explicit compatibility-shim references.

- [ ] **Step 5: Run full verification**

Run:

```bash
cargo check --workspace
cargo test -p auki-proto
cargo test -p auki-datatypes
just generate-proto
git status --short bindings/javascript/auki-proto bindings/swift/auki-proto bindings/python/auki-proto
```

Expected: cargo commands pass; `just generate-proto` refreshes Rust plus local non-Rust outputs; the final `git status --short` command prints no output for ignored non-Rust generated directories.

- [ ] **Step 6: Add final docs changelog entries**

Prepend to `docs/changelog.md` if Superpowers docs are updated during implementation. Prepend root `changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

**`auki-proto` protobuf generation migration documented and propagated.** Active docs now point schema/codegen readers at root `proto/auki` plus generated platform packages, while `auki-datatypes` is documented as a deprecated Rust compatibility shim.
```

- [ ] **Step 7: Commit**

Run:

```bash
git add README.md crates bindings docs changelog.md
git commit -m "docs: document auki-proto migration"
```

## Task 8: First New Schema Uses `auki-proto`

**Files:**
- Create: `proto/auki/message.proto`
- Regenerate: `crates/auki-proto/src/generated/auki.message.rs`
- Generate locally, do not commit: `bindings/javascript/auki-proto/`
- Generate locally, do not commit: `bindings/swift/auki-proto/`
- Generate locally, do not commit: `bindings/python/auki-proto/`
- Modify: Rust locked-vector tests
- Modify: changelogs

- [ ] **Step 1: Add message schema**

Create `proto/auki/message.proto`:

```proto
syntax = "proto3";

package auki.message;

message MessageEnvelope {
  string type_url = 1;
  bytes body = 2;
  string request_id = 3;
}

message MessageAck {
  string request_id = 1;
  bool accepted = 2;
  string detail = 3;
}
```

- [ ] **Step 2: Update Rust generator proto list**

Add `"message.proto"` to `tools/proto-gen-rust/src/main.rs` `PROTOS`.

Add module include to `crates/auki-proto/src/lib.rs`:

```rust
pub mod message {
    include!("generated/auki.message.rs");
}
```

- [ ] **Step 3: Regenerate all platform outputs**

Run:

```bash
just generate-proto
```

Expected: Rust generated files under `crates/auki-proto` contain `message` bindings. JavaScript/TypeScript, Swift, and Python generated files under `bindings/*/auki-proto/` also contain `message` bindings locally but remain ignored by git.

- [ ] **Step 4: Add Rust locked vectors**

Add Rust locked-vector tests for:

```rust
#[test]
fn message_envelope_locked_vector() {
    let envelope = auki_proto::message::MessageEnvelope {
        type_url: "auki.test/hello".to_string(),
        body: vec![1, 2, 3],
        request_id: "req-1".to_string(),
    };

    assert_eq!(
        hex::encode(envelope.encode_to_vec()),
        "0a0f61756b692e746573742f68656c6c6f12030102031a057265712d31"
    );
}

#[test]
fn message_ack_locked_vector() {
    let ack = auki_proto::message::MessageAck {
        request_id: "req-1".to_string(),
        accepted: true,
        detail: "ok".to_string(),
    };

    assert_eq!(
        hex::encode(ack.encode_to_vec()),
        "0a057265712d3110011a026f6b"
    );
}
```

- [ ] **Step 5: Verify Rust and local generated outputs**

Run:

```bash
cargo test -p auki-proto
find bindings/javascript/auki-proto -iname '*message*' -print -quit
find bindings/swift/auki-proto -iname '*message*' -print -quit
find bindings/python/auki-proto -iname '*message*' -print -quit
git status --short bindings/javascript/auki-proto bindings/swift/auki-proto bindings/python/auki-proto
```

Expected: Rust locked-vector tests pass; each `find` command prints one generated file path; `git status --short` prints no output for non-Rust generated directories.

- [ ] **Step 6: Commit**

Run:

```bash
git add proto/auki/message.proto tools/proto-gen-rust/src/main.rs crates/auki-proto/src/lib.rs crates/auki-proto/src/generated/auki.message.rs crates/auki-proto/tests crates/changelog.md changelog.md
git commit -m "feat: add auki message proto schema"
```

Do not add `bindings/javascript/auki-proto`, `bindings/swift/auki-proto`, or `bindings/python/auki-proto`.

## Self-Review

Spec coverage:

- Cross-platform proto generation is covered by Tasks 2 and 6. Rust generated output is committed; JavaScript/TypeScript, Swift, and Python generated outputs are local ignored binding artifacts.
- Rust follows the same generated-package path via `crates/auki-proto`, covered by Tasks 2 and 3.
- `auki-datatypes` deprecation is covered by Task 4.
- Workspace migration is covered by Task 5.
- Documentation propagation is covered by Task 7.
- The first new schema using `auki-proto` is covered by Task 8.

Placeholder scan:

- No step uses unspecified implementation language such as "handle edge cases" without concrete commands or file edits.
- The only deferred behavior is the separate `/auki/message/0.0.1` transport handler, which is intentionally outside this proto-generation migration.

Type consistency:

- Rust crate name: `auki-proto`; Rust import path: `auki_proto`.
- JavaScript generated output path: `bindings/javascript/auki-proto`; local package name: `@aukilabs/auki-proto`; ignored by git.
- Swift generated output path: `bindings/swift/auki-proto`; local product name: `AukiProto`; ignored by git.
- Python generated output path: `bindings/python/auki-proto`; local project name: `auki-proto`; ignored by git.
