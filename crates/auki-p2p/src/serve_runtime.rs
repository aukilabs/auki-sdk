//! SDK-owned serving runtime for inbound protocol traffic.

use crate::api::{AukiNode, AukiNodeError, AukiServedInbound, LifecycleInput};

/// Lightweight counters for the SDK serving loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AukiServeRuntimeStatus {
    /// Lifecycle handshakes served.
    pub lifecycles_served: u64,
    /// Offer-catalog requests served.
    pub offer_catalogs_served: u64,
    /// Successful Get requests served.
    pub gets_served: u64,
    /// Failed Get requests served with a structured protocol response.
    pub gets_rejected: u64,
    /// Subscribe start requests accepted.
    pub subscriptions_accepted: u64,
    /// Subscribe start requests rejected with a structured protocol response.
    pub subscriptions_rejected: u64,
}

impl AukiServeRuntimeStatus {
    fn record_inbound(&mut self, served: &AukiServedInbound) {
        match served {
            AukiServedInbound::Lifecycle(_) => {
                self.lifecycles_served = self.lifecycles_served.saturating_add(1);
            }
            AukiServedInbound::OfferCatalog(_) => {
                self.offer_catalogs_served = self.offer_catalogs_served.saturating_add(1);
            }
            AukiServedInbound::Get(served) => {
                if served.success {
                    self.gets_served = self.gets_served.saturating_add(1);
                } else {
                    self.gets_rejected = self.gets_rejected.saturating_add(1);
                }
            }
            AukiServedInbound::Subscribe(served) => {
                if served.accepted {
                    self.subscriptions_accepted = self.subscriptions_accepted.saturating_add(1);
                } else {
                    self.subscriptions_rejected = self.subscriptions_rejected.saturating_add(1);
                }
            }
        }
    }
}

/// Runtime wrapper that owns one [`AukiNode`] serving inbound SDK protocols.
pub struct AukiServeRuntime {
    node: AukiNode,
    lifecycle_input: LifecycleInput,
    status: AukiServeRuntimeStatus,
}

impl AukiServeRuntime {
    /// Create a serving runtime around an already configured node.
    pub fn new(node: AukiNode) -> Self {
        Self {
            node,
            lifecycle_input: LifecycleInput::new(),
            status: AukiServeRuntimeStatus::default(),
        }
    }

    /// Override the lifecycle policy input used for inbound handshakes.
    pub fn with_lifecycle_input(mut self, lifecycle_input: LifecycleInput) -> Self {
        self.lifecycle_input = lifecycle_input;
        self
    }

    /// Borrow the owned node for diagnostics or local provider registration.
    pub fn node(&self) -> &AukiNode {
        &self.node
    }

    /// Mutably borrow the owned node for configuration before the loop runs.
    pub fn node_mut(&mut self) -> &mut AukiNode {
        &mut self.node
    }

    /// Consume the runtime and return the owned node.
    pub fn into_node(self) -> AukiNode {
        self.node
    }

    /// Return the current serving counters.
    pub fn status(&self) -> &AukiServeRuntimeStatus {
        &self.status
    }

    /// Serve one ready inbound protocol stream without fixed per-protocol timeout sequencing.
    pub async fn serve_next(
        &mut self,
        now: &str,
    ) -> Result<Option<AukiServedInbound>, AukiNodeError> {
        let served = self
            .node
            .serve_next_inbound(self.lifecycle_input.clone(), now)
            .await?;
        if let Some(served) = &served {
            self.status.record_inbound(served);
        }
        Ok(served)
    }
}
