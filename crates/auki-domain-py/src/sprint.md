# Sprint - auki-domain-py

## Current

The Python binding tracks the Hagall `ClusterManager` surface:

- `ClusterTarget` + `ClusterManager.bootstrap` for SDK-fronted cluster selection.
- `create_cluster` / `join_cluster` explicit operator-intent paths.
- Stream producer support through `stream_provider`.
- Stream consumer methods for JPEG, point cloud, joint encoders, and audio.
- Info and sensor catalog exchange helpers.
- `external_addresses` override for Discovery advertisement.

The old `init_domain` / `DomainHandle` shape is gone. Any README or consumer code still referring to it is stale.

## Next

- Improve Python exception specificity if downstream apps need typed catches beyond the current built-in exception mapping.
- Mirror any future Rust relay-reservation helper once `auki-network` ships it.
- Keep surface pins updated as Park and Boosterapp complete the `ClusterManager.bootstrap` migration.

## Long-term

Stay thin: this crate should remain a Python-shaped facade over `auki-domain`, with stream pyclasses owned by `auki-network-py`.
