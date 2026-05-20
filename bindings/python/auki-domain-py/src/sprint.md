# Sprint - auki-domain-py

## Current

The Python binding tracks the Hagall `ClusterManager` surface:

- `ClusterTarget` + `ClusterManager.bootstrap` for SDK-fronted cluster selection.
- `create_cluster` / `join_cluster` explicit operator-intent paths.
- Stream producer support through `stream_provider`.
- Stream consumer methods for JPEG, point cloud, joint encoders, and audio.
- `StreamManifestBuilder.from_registry` for producer accept metadata.
- Info and sensor catalog exchange helpers.
- Registry serving root registration via `set_registry_app_root`.
- Exact registry-entry JSON fetch helpers and opt-in sensor catalog detail embedding.
- `external_addresses` override for Discovery advertisement.

The old `init_domain` / `DomainHandle` shape is gone. Any README or consumer code still referring to it is stale.

## Next

- Improve Python exception specificity if downstream apps need typed catches beyond the current built-in exception mapping.
- Mirror any future Rust relay-reservation helper once `auki-network` ships it.
- Keep surface pins updated as Park and Boosterapp complete the `ClusterManager.bootstrap` migration.
- Add Python-side happy-path tests for `StreamManifestBuilder.from_registry` once the wheel test harness installs `auki_network` beside `auki_domain`; the Rust PyO3 test already pins the cross-module type identity.

## Long-term

Stay thin: this crate should remain a Python-shaped facade over `auki-domain`, with stream pyclasses owned by `auki-network-py`.
