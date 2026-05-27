//! Session — per-process declarative API.
//!
//! See `crate::Session` (and `docs/superpowers/specs/.../#section-§4`).

use std::sync::Arc;
use parking_lot::RwLock;

pub struct Session {
    pub(crate) inner: Arc<RwLock<SessionInner>>,
}

pub(crate) struct SessionInner {
    pub(crate) peer_id: String,
    pub(crate) app_id: String,
    pub(crate) session_id: String,
}
