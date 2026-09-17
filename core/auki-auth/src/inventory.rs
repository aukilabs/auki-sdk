//! Typed DDS inventory reads, sharing the session's refresh and persistence.
use super::*;
use serde::Serialize;

const INVENTORY: &str = "DDS machine inventory";

/// Only inventory metadata is retained; credentials, addresses and wallet fields
/// from the generic DDS node response are deliberately not exposed here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InventoryNode {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub capabilities: Vec<String>,
    pub mode: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InventoryRobot {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub assigned_domain_id: Option<Uuid>,
    pub capabilities: Vec<String>,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub active_lease_expires_at: Option<DateTime<Utc>>,
}

impl AuthSession {
    /// All visible public and own dedicated nodes, including unstaked/offline
    /// entries. DDS has no pagination for this response; AuthLimits bounds it.
    /// None means the imported grant cannot safely use this broad inventory.
    pub async fn inventory_nodes(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<InventoryNode>>> {
        let mut url = self.inner.client.dds_url("api/v1/nodes");
        url.query_pairs_mut()
            .append_pair("org", "all")
            .append_pair("staking_status", "all");
        #[derive(Deserialize)]
        struct Nodes {
            nodes: Vec<InventoryNode>,
        }
        let Some(response) = self
            .inventory_json::<Nodes>(url, None, cancellation)
            .await?
        else {
            return Ok(None);
        };
        let mut ids = HashSet::new();
        if response
            .nodes
            .iter()
            .any(|node| node.id.is_nil() || node.organization_id.is_nil() || !ids.insert(node.id))
        {
            return Err(Error::invalid_response(
                INVENTORY,
                "invalid or duplicate node identity",
            ));
        }
        Ok(Some(response.nodes))
    }

    /// Assignment inventory only. Listing robots does not grant task authority.
    /// None means the imported token profile is unsupported by this DDS route.
    pub async fn inventory_robots(
        &self,
        domain_id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<InventoryRobot>>> {
        if domain_id.is_nil() {
            return Err(Error::InvalidInput {
                field: "Domain ID",
                reason: "expected non-nil UUID",
            });
        }
        let url = self
            .inner
            .client
            .dds_url(&format!("api/v1/domains/{domain_id}/robots"));
        #[derive(Deserialize)]
        struct Robots {
            robots: Vec<InventoryRobot>,
        }
        let Some(response) = self
            .inventory_json::<Robots>(url, Some(domain_id), cancellation)
            .await?
        else {
            return Ok(None);
        };
        let mut ids = HashSet::new();
        if response.robots.iter().any(|robot| {
            robot.id.is_nil()
                || robot.organization_id.is_nil()
                || robot.assigned_domain_id != Some(domain_id)
                || !ids.insert(robot.id)
        }) {
            return Err(Error::invalid_response(
                INVENTORY,
                "invalid, duplicate or wrong-Domain robot",
            ));
        }
        Ok(Some(response.robots))
    }

    async fn inventory_json<T: DeserializeOwned>(
        &self,
        url: Url,
        domain: Option<Uuid>,
        cancellation: &CancellationToken,
    ) -> Result<Option<T>> {
        let operation = async {
            let mut state = self.lock_state(cancellation, INVENTORY).await?;
            let PrincipalState::Zitadel(session) = &state.principal else {
                return self
                    .data_dds_json(&mut state, url, false, INVENTORY, cancellation)
                    .await
                    .map(Some);
            };
            let mut ready = session.ready(RefreshMode::IfExpiring, cancellation).await?;
            let mut refresh_used = ready.refreshed;
            for attempt in 0..2 {
                let grant = loop {
                    let request = self
                        .inner
                        .client
                        .inner
                        .http
                        .post(self.inner.client.api_url("service/domains-access-token"))
                        .header(ACCEPT, "application/json")
                        .bearer_auth(ready.credentials.access_token().expose());
                    match self
                        .inner
                        .client
                        .send_json::<ServiceTokenResponse>(request, API_SERVICE_TOKEN, cancellation)
                        .await
                    {
                        Err(error) if error.is_unauthorized() => {
                            if refresh_used {
                                session.require_login().await;
                                return Err(Error::AuthenticationRequired);
                            }
                            ready = session.ready(RefreshMode::Force, cancellation).await?;
                            refresh_used = true;
                        }
                        result => {
                            break validate_imported_ordinary_listing_profile(
                                result?.access_token,
                            )?;
                        }
                    }
                };
                // In particular, never send a human viewer's App-shaped token
                // to these legacy organization-wide routes.
                let ImportedOrdinaryProfile::User(grant) = grant else {
                    return Ok(None);
                };
                if !matches!(grant.kind, ImportedListingGrantKind::User { organization } if !organization.is_nil())
                {
                    return Err(Error::invalid_response(
                        INVENTORY,
                        "invalid inventory organization",
                    ));
                }
                if !grant.domains.is_empty() {
                    let Some(domain) = domain else {
                        return Ok(None);
                    };
                    if !grant.domains.contains(&domain) {
                        return Err(Error::DomainNotAccessible);
                    }
                }
                let request = self
                    .inner
                    .client
                    .inner
                    .http
                    .get(url.clone())
                    .bearer_auth(grant.bearer.expose())
                    .header(ACCEPT, "application/json")
                    .header("posemesh-client-id", self.client_id())
                    .header(
                        "posemesh-sdk-version",
                        concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                    );
                match self
                    .inner
                    .client
                    .send_json(request, INVENTORY, cancellation)
                    .await
                {
                    Err(error) if attempt == 0 && error.is_unauthorized() => continue,
                    result => return result.map(Some),
                }
            }
            unreachable!("second attempt always returns")
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            _ = cancellation.cancelled() => Err(Error::Cancelled { endpoint: INVENTORY }),
            result = operation => result,
        }
    }
}
