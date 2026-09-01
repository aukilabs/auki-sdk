//! Shared mechanics for the finite Swift protocol adapters.

use std::{future::Future, pin::Pin};

use auki_sdk_rs::{Multiaddr, PeerId};
use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    AukiPeerTarget, AukiSdkError, CleanupResult, DetachedCleanup, operation_error, parse_target,
};

pub(crate) type CloseFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;
type CloseFunction<E> = fn(E) -> CloseFuture;

/// Own one consuming Rust endpoint behind an idempotent detached close barrier.
pub(crate) struct EndpointOwner<E: Send + 'static> {
    endpoint: Mutex<Option<E>>,
    cleanup: DetachedCleanup,
    close: CloseFunction<E>,
}

impl<E: Send + 'static> EndpointOwner<E> {
    pub(crate) fn new(endpoint: E, close: CloseFunction<E>) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            cleanup: DetachedCleanup::new(),
            close,
        }
    }

    pub(crate) fn ensure_open(&self, component: &'static str) -> Result<(), AukiSdkError> {
        if self.endpoint.lock().is_some() {
            Ok(())
        } else {
            Err(operation_error(component, "endpoint is stopped"))
        }
    }

    pub(crate) fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let endpoint = self.endpoint.lock().take();
            let close = self.close;
            async move {
                match endpoint {
                    Some(endpoint) => close(endpoint).await,
                    None => Ok(()),
                }
            }
        })
    }
}

impl<E: Send + 'static> Drop for EndpointOwner<E> {
    fn drop(&mut self) {
        if self.endpoint.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

/// Validate Domain equality before parsing one exact Peer ID and route.
pub(crate) fn exact_target(
    local_domain_id: &str,
    target: AukiPeerTarget,
) -> Result<(PeerId, Multiaddr), AukiSdkError> {
    let local_domain = Uuid::parse_str(local_domain_id)
        .map_err(|error| operation_error("parse local Auki Domain ID", error))?;
    let target_domain = Uuid::parse_str(&target.domain_id)
        .map_err(|error| operation_error("parse target Auki Domain ID", error))?;
    if local_domain != target_domain {
        return Err(operation_error(
            "validate exact Auki peer target",
            format!(
                "target Domain {} does not match local Domain {}",
                target.domain_id, local_domain_id
            ),
        ));
    }
    parse_target(target)
}

/// Decode one bounded canonical protocol JSON value.
pub(crate) fn parse_bounded_json<T: DeserializeOwned>(
    context: &'static str,
    json: &str,
    maximum_bytes: usize,
) -> Result<T, AukiSdkError> {
    if json.len() > maximum_bytes {
        return Err(operation_error(
            context,
            format!("JSON is {} bytes; maximum is {maximum_bytes}", json.len()),
        ));
    }
    serde_json::from_str(json).map_err(|error| operation_error(context, error))
}

/// Encode one validated value as compact bounded canonical-shape JSON.
pub(crate) fn bounded_json<T: Serialize>(
    context: &'static str,
    value: &T,
    maximum_bytes: usize,
) -> Result<String, AukiSdkError> {
    let json = serde_json::to_string(value).map_err(|error| operation_error(context, error))?;
    if json.len() > maximum_bytes {
        return Err(operation_error(
            context,
            format!(
                "encoded JSON is {} bytes; maximum is {maximum_bytes}",
                json.len()
            ),
        ));
    }
    Ok(json)
}

#[cfg(test)]
pub(crate) fn authenticated_peer(peer_id: PeerId) -> auki_sdk_rs::AuthenticatedPeer {
    auki_sdk_rs::AuthenticatedPeer {
        peer_id,
        subject: "b03a67cb-45d4-4f60-a8b8-d9687e91d018".parse().unwrap(),
        peer_type: Some("native_app".into()),
        domain_ids: vec!["4e990513-b110-467b-84ca-09a42d786f6d".parse().unwrap()],
        scopes: vec!["protocols:read".into()],
        application: None,
        verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        value: u64,
    }

    #[test]
    fn bounded_json_rejects_oversize_and_unknown_fields() {
        assert_eq!(
            parse_bounded_json::<Fixture>("read fixture", r#"{"value":7}"#, 32).unwrap(),
            Fixture { value: 7 }
        );
        assert!(
            parse_bounded_json::<Fixture>("read fixture", r#"{"value":7,"unexpected":true}"#, 64,)
                .is_err()
        );
        assert!(parse_bounded_json::<Fixture>("read fixture", r#"{"value":7}"#, 2).is_err());
        assert!(bounded_json("write fixture", &Fixture { value: 7 }, 2).is_err());
    }

    #[test]
    fn exact_target_requires_the_same_domain_before_route_parsing() {
        let target = AukiPeerTarget {
            domain_id: "00000000-0000-0000-0000-000000000002".into(),
            peer_id: "not-a-peer".into(),
            route: "not-a-route".into(),
        };
        let error = exact_target("00000000-0000-0000-0000-000000000001", target)
            .expect_err("different Domains must fail");
        assert!(error.to_string().contains("does not match local Domain"));
    }
}
