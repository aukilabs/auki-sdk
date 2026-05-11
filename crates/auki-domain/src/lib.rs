//! Domain lifecycle for the Auki SDK.
//!
//! A **Domain** is the unit of cluster identity — the topic peers cluster
//! around on the network, and (per [`Glossary.md`](../../Glossary.md)) the
//! tag that asserts data describes a specific physical space. This crate
//! owns Domain *lifecycle*: creating a Domain (`init_domain`), joining an
//! existing one (`join_domain`), the Manager/Member roles, heartbeats,
//! the live Cluster Registry, and Manager failover.
//!
//! It is **not** the home for `convert_time` / `convert_pose` — those
//! operate inside a Domain but live elsewhere. It is also not the home
//! for log-writing session lifecycle (sensor logs, pose logs, registry
//! entries) — that's [`auki-session-py`](../auki-session-py)'s eventual
//! Rust sibling.
//!
//! ## Status
//!
//! Scaffolding only. No functional code. See [`src/readme.md`](readme.md)
//! for current state, [`src/sprint.md`](sprint.md) for the Greenland PR
//! sequence (PR 1 lands `DomainIdentity` + `init_domain`).
//!
//! ## Aspirational surface
//!
//! See [`README.md`](../README.md) for the target API shape — what
//! consumers eventually call.

// (Empty — first implementation lands in Greenland PR 1.)
