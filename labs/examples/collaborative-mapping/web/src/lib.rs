//! Browser-only adapter. The same WASM module owns SDK handles and Map Components.
#![cfg(target_arch = "wasm32")]
#![forbid(unsafe_code)]
use auki_collaborative_mapping::{
    Convention, DEMO_VERSION, DemoMap, PublishedMap, definition_with_convention, discover_map,
    map_convention,
};
use auki_component_protocol::{
    CATALOG_PROTOCOL_ID, CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint,
    ObservationStart, RemoteObservationEvent,
};
use auki_components::{BufferLimits, ProductReference};
use auki_scenegraph::{MAX_SNAPSHOT_BYTES, MapSnapshot, component::SnapshotReference};
use auki_sdk::{Multiaddr, PeerId};
pub use auki_sdk_web::{AukiDiscoveryMode, AukiPeer, AukiPeerReachabilityMode, AukiUserSession};
use futures::{FutureExt, select_biased};
use js_sys::{Function, Promise};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, time::Duration};
use tokio_util::sync::CancellationToken;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionCard {
    version: String,
    convention: Convention,
    domain: String,
    session: String,
    peer: String,
    route: String,
    product: ProductReference,
}
struct State {
    model: RefCell<DemoMap>,
    endpoint: RefCell<Option<ComponentProtocolEndpoint>>,
    client: ComponentProtocolClient,
    card: ConnectionCard,
    cancel: CancellationToken,
    follows: RefCell<BTreeMap<String, (CancellationToken, Promise)>>,
    closing: RefCell<Option<Promise>>,
}
#[wasm_bindgen]
pub struct AukiMapping {
    state: Rc<State>,
}

#[wasm_bindgen]
impl AukiMapping {
    #[wasm_bindgen]
    pub async fn mount(
        peer: &AukiPeer,
        session: String,
        convention: String,
    ) -> Result<AukiMapping, JsValue> {
        let route = peer
            .wss_route()
            .ok_or_else(|| error("A confirmed WSS relay route is required"))?;
        let protocols = peer.protocols().ok_or_else(|| error("Peer has stopped"))?;
        let convention = Convention::parse(&convention).map_err(error)?;
        let model = DemoMap::with_convention(
            peer.peer_id(),
            peer.domain_id(),
            session.clone(),
            convention,
        )
        .map_err(error)?;
        let endpoint = ComponentProtocolEndpoint::mount(protocols.clone(), model.runtime.clone())
            .map_err(error)?;
        if let Err(e) = endpoint.export_product(&model.map.product()) {
            let _ = endpoint.close().await;
            return Err(error(e));
        }
        let card = ConnectionCard {
            convention,
            version: DEMO_VERSION.into(),
            domain: peer.domain_id(),
            session,
            peer: peer.peer_id(),
            route,
            product: model.map.product().reference(),
        };
        Ok(Self {
            state: Rc::new(State {
                model: RefCell::new(model),
                endpoint: RefCell::new(Some(endpoint)),
                client: ComponentProtocolClient::new(protocols),
                card,
                cancel: CancellationToken::new(),
                follows: RefCell::new(BTreeMap::new()),
                closing: RefCell::new(None),
            }),
        })
    }
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        CATALOG_PROTOCOL_ID.into()
    }

    /// Inspect one fresh DDS candidate over its exact authenticated route. No session match is a
    /// normal absence; discovery hints never bypass signed P2P authorization or snapshot checks.
    #[wasm_bindgen(js_name = inspectPeer)]
    pub async fn inspect_peer(
        &self,
        peer: String,
        route: String,
    ) -> Result<Option<String>, JsValue> {
        if self.state.cancel.is_cancelled() {
            return Err(error("Map is closing"));
        }
        if peer == self.state.card.peer {
            return Ok(None);
        }
        let expected: PeerId = peer.parse().map_err(error)?;
        let address = relay_route(&route)?;
        let catalog = select_biased! {
            _ = self.state.cancel.cancelled().fuse() => return Err(error("Discovery canceled")),
            result = self.state.client.catalog_exact(expected, address, None).fuse() => result.map_err(error)?,
        };
        let CatalogResponse::Snapshot { snapshot } = catalog else {
            return Err(error("Candidate Catalog unavailable"));
        };
        let Some(product) = discover_map(
            &snapshot,
            &peer,
            &self.state.card.domain,
            &self.state.card.session,
        )
        .map_err(error)?
        else {
            return Ok(None);
        };
        let metadata = snapshot
            .products
            .iter()
            .find(|p| p.manifest.reference() == product)
            .and_then(|p| p.metadata.as_ref())
            .ok_or_else(|| error("Missing map convention"))?;
        let data: auki_scenegraph::catalog::MapCatalogData =
            serde_json::from_value(metadata.value.clone()).map_err(error)?;
        let convention = map_convention(
            &data.map,
            &peer,
            &self.state.card.domain,
            &self.state.card.session,
        )
        .map_err(error)?;
        let selected = ConnectionCard {
            convention,
            version: DEMO_VERSION.into(),
            domain: self.state.card.domain.clone(),
            session: self.state.card.session.clone(),
            peer,
            route,
            product,
        };
        serde_json::to_string(&selected).map(Some).map_err(error)
    }
    pub fn view(&self) -> Result<String, JsValue> {
        view(&self.state)
    }
    pub fn place(&self, name: String, x: f64, y: f64, frame: String) -> Result<String, JsValue> {
        if self.state.cancel.is_cancelled() {
            return Err(error("Map is closing"));
        }
        self.state
            .model
            .borrow_mut()
            .place(&name, x, y, &frame)
            .map_err(error)?;
        view(&self.state)
    }
    pub fn remove(&self, name: String) -> Result<String, JsValue> {
        if self.state.cancel.is_cancelled() {
            return Err(error("Map is closing"));
        }
        self.state.model.borrow_mut().remove(&name).map_err(error)?;
        view(&self.state)
    }
    /// One independently driven subscription per peer; callers await this Promise during shutdown.
    /// Callbacks carry serialized view state and human-readable transport state.
    pub fn follow(
        &self,
        card: String,
        changed: Function,
        status: Function,
    ) -> Result<Promise, JsValue> {
        if self.state.cancel.is_cancelled() {
            return Err(error("Map is closed"));
        }
        if card.len() > 8192 {
            return Err(error("Discovered map selection is too large"));
        }
        let card: ConnectionCard =
            serde_json::from_str(&card).map_err(|_| error("Invalid discovered map selection"))?;
        if card.version != DEMO_VERSION
            || card.session != self.state.card.session
            || card.domain != self.state.card.domain
            || card.peer == self.state.card.peer
            || card.product.peer_id != card.peer
        {
            return Err(error(
                "Discovered map must belong to another peer in the same version, session and Domain",
            ));
        }
        if self.state.follows.borrow().contains_key(&card.peer) {
            return Err(error("Already following this peer"));
        }
        if self.state.follows.borrow().len() >= 15 {
            return Err(error("Session supports at most 16 peers"));
        }
        let peer: PeerId = card.peer.parse().map_err(error)?;
        let route = relay_route(&card.route)?;
        self.state
            .model
            .borrow_mut()
            .select_partner(card.product.clone())
            .map_err(error)?;
        let cancel = self.state.cancel.child_token();
        let task_cancel = cancel.clone();
        let key = card.peer.clone();
        let task_key = key.clone();
        let state = Rc::clone(&self.state);
        let promise = future_to_promise(async move {
            let result = async {
                select_biased! {
                    _ = task_cancel.cancelled().fuse() => Ok(JsValue::UNDEFINED),
                    result = receive_loop(&state, card, peer, route, &changed, &status).fuse() => result,
                }
            }.await;
            state.follows.borrow_mut().remove(&task_key);
            state.model.borrow_mut().forget_peer(&task_key);
            if !state.cancel.is_cancelled() {
                call(&changed, &view(&state)?)?;
            }
            result
        });
        self.state
            .follows
            .borrow_mut()
            .insert(key, (cancel, promise.clone()));
        Ok(promise)
    }
    /// Cancel a departed or replaced publication and await its receive loop.
    pub fn unfollow(&self, peer: String) -> Promise {
        let entry = self.state.follows.borrow().get(&peer).cloned();
        future_to_promise(async move {
            if let Some((cancel, promise)) = entry {
                cancel.cancel();
                let _ = JsFuture::from(promise).await;
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    /// Idempotent close joins the receive task before unexporting; peer shutdown is owned by UI.
    pub fn close(&self) -> Promise {
        if let Some(promise) = self.state.closing.borrow().as_ref() {
            return promise.clone();
        }
        self.state.cancel.cancel();
        let state = Rc::clone(&self.state);
        let promise = future_to_promise(async move {
            let tasks: Vec<_> = state
                .follows
                .borrow()
                .values()
                .map(|(_, task)| task.clone())
                .collect();
            for task in tasks {
                let _ = JsFuture::from(task).await;
            }
            let endpoint = state.endpoint.borrow_mut().take();
            let result = if let Some(endpoint) = endpoint {
                endpoint.close().await.map_err(error)
            } else {
                Ok(())
            };
            state.model.borrow_mut().close();
            result?;
            Ok(JsValue::UNDEFINED)
        });
        self.state.closing.replace(Some(promise.clone()));
        promise
    }
}
impl Drop for AukiMapping {
    fn drop(&mut self) {
        self.state.cancel.cancel();
    }
}
fn view(state: &State) -> Result<String, JsValue> {
    serde_json::to_string(&state.model.borrow_mut().session_view().map_err(error)?).map_err(error)
}
fn call(function: &Function, value: &str) -> Result<(), JsValue> {
    function
        .call1(&JsValue::UNDEFINED, &JsValue::from_str(value))
        .map(|_| ())
}
async fn verify_catalog(
    state: &State,
    card: &ConnectionCard,
    peer: PeerId,
    route: &Multiaddr,
) -> Result<Option<u64>, JsValue> {
    let catalog = state
        .client
        .catalog_exact(peer, route.clone(), None)
        .await
        .map_err(error)?;
    let CatalogResponse::Snapshot { snapshot } = catalog else {
        return Err(error("Partner Catalog unavailable"));
    };
    let entry = snapshot
        .products
        .iter()
        .find(|p| {
            p.manifest.reference() == card.product && p.manifest_hash == card.product.manifest_hash
        })
        .ok_or_else(|| error("Selected map is no longer exported; refreshing discovery"))?;
    Ok(entry
        .metadata
        .as_ref()
        .map(|metadata| metadata.source_sequence))
}
async fn receive_loop(
    state: &State,
    card: ConnectionCard,
    peer: PeerId,
    route: Multiaddr,
    changed: &Function,
    status: &Function,
) -> Result<JsValue, JsValue> {
    // Total attempts are bounded, including after long-lived connections. The user can pair again.
    for attempt in 0..3u32 {
        call(
            status,
            if attempt == 0 {
                "Connecting"
            } else {
                "Reconnecting · remote map is stale"
            },
        )?;
        if attempt > 0 {
            futures_timer::Delay::new(Duration::from_secs(1 << (attempt - 1))).await;
        }
        let result = async {
            verify_catalog(state, &card, peer, &route).await?;
            let mut subscription = state
                .client
                .subscribe_product_exact::<MapSnapshot>(
                    peer,
                    route.clone(),
                    card.product.clone(),
                    ObservationStart::LatestExisting,
                    BufferLimits {
                        max_entries: Some(1),
                        max_bytes: Some(MAX_SNAPSHOT_BYTES),
                        target_duration: None,
                    },
                    |snapshot| {
                        serde_json::to_vec(snapshot).map_or(MAX_SNAPSHOT_BYTES + 1, |v| v.len())
                    },
                )
                .await
                .map_err(error)?;
            let expected = definition_with_convention(&card.peer, &card.domain, &card.session, card.convention);
            let producer = &subscription.product().producer;
            if producer.spatial_frame_id.as_deref() != Some(expected.frame.id.as_str())
                || producer.payload.schema() != auki_scenegraph::SNAPSHOT_SCHEMA
                || producer.payload.observes() != expected.map_id
            {
                return Err(error(
                    "Remote Product does not declare the selected map and frame",
                ));
            }
            loop {
                let event = select_biased! {
                    event = subscription.next().fuse() => event.map_err(error)?,
                    _ = futures_timer::Delay::new(Duration::from_secs(15)).fuse() => {
                        // A bounded Catalog probe detects silent partitions without timing out idle maps.
                        let advertised = verify_catalog(state, &card, peer, &route).await?;
                        if advertised.is_some_and(|sequence| sequence >= subscription.next_sequence()) {
                            // A healthy Catalog alone does not prove that the observation stream is live.
                            let _ = subscription.close().await;
                            return Err(error("Map stream fell behind its Catalog; reopening latest snapshot"));
                        }
                        call(status, "Live · up to date")?;
                        continue;
                    }
                };
                match event {
                    Some(RemoteObservationEvent::Observation(observation)) => {
                        state
                            .model
                            .borrow_mut()
                            .receive(PublishedMap {
                                reference: SnapshotReference {
                                    product: card.product.clone(),
                                    sequence: observation.sequence,
                                },
                                snapshot: (*observation.payload).clone(),
                            })
                            .map_err(error)?;
                        call(changed, &view(state)?)?;
                        call(status, "Connected · snapshots flowing over relay")?;
                    }
                    Some(RemoteObservationEvent::Gap(_)) => {
                        call(status, "Catching up to latest complete map")?;
                    }
                    Some(RemoteObservationEvent::Closed(_)) | None => {
                        let _ = subscription.close().await;
                        return Err(error(
                            "Partner stopped publishing; refresh discovery after restart",
                        ));
                    }
                }
            }
        }
        .await;
        if let Err(ref failure) = result {
            state.model.borrow_mut().clear_evidence(&card.peer);
            call(changed, &view(state)?)?;
            let message = js_sys::Reflect::get(failure, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "Connection interrupted".into());
            call(status, &format!("Reconnecting: {message}"))?;
        }
        if attempt == 2 {
            return result;
        }
    }
    unreachable!()
}
fn relay_route(route: &str) -> Result<Multiaddr, JsValue> {
    let address: Multiaddr = route.parse().map_err(error)?;
    if (!route.contains("/wss") && !route.contains("/tls/ws")) || !route.contains("/p2p-circuit/") {
        return Err(error("Partner route must use a WSS relay circuit"));
    }
    Ok(address)
}
fn error(value: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&value.to_string()).into()
}

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
#[cfg(test)]
#[path = "../../tests/mapping.rs"]
mod mapping_tests;
