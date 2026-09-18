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

/// One bounded provider page, or a bounded legacy complete response.
/// `paginated` distinguishes DDS acknowledgement from ignored query parameters.
#[derive(Clone, Debug)]
pub struct InventoryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub paginated: bool,
}

#[derive(Deserialize)]
struct InventoryPagination {
    version: u32,
    limit: usize,
    next_cursor: String,
}

fn inventory_page<T>(
    items: Vec<T>,
    pagination: Option<InventoryPagination>,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<InventoryPage<T>> {
    let Some(pagination) = pagination else {
        if cursor.is_some() {
            return Err(Error::invalid_response(
                INVENTORY,
                "provider stopped acknowledging pagination",
            ));
        }
        return Ok(InventoryPage {
            items,
            next_cursor: None,
            paginated: false,
        });
    };
    if pagination.version != 1
        || Some(pagination.limit) != limit
        || items.len() > pagination.limit
        || pagination.next_cursor.len() > 2048
        || (!pagination.next_cursor.is_empty() && items.is_empty())
    {
        return Err(Error::invalid_response(
            INVENTORY,
            "invalid inventory pagination",
        ));
    }
    let next_cursor = (!pagination.next_cursor.is_empty()).then_some(pagination.next_cursor);
    Ok(InventoryPage {
        items,
        next_cursor,
        paginated: true,
    })
}

fn inventory_page_query(url: &mut Url, limit: Option<usize>, cursor: Option<&str>) -> Result<()> {
    if let Some(limit) = limit {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput {
                field: "inventory limit",
                reason: "expected 1 through 100",
            });
        }
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string());
    }
    if let Some(cursor) = cursor {
        if cursor.is_empty() || cursor.len() > 2048 {
            return Err(Error::InvalidInput {
                field: "inventory cursor",
                reason: "expected 1 through 2048 bytes",
            });
        }
        url.query_pairs_mut().append_pair("cursor", cursor);
    }
    Ok(())
}

impl AuthSession {
    /// Legacy complete-list read, bounded by AuthLimits. Use
    /// `inventory_nodes_page` to request server pagination.
    /// None means the imported grant cannot safely use this broad inventory.
    pub async fn inventory_nodes(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<InventoryNode>>> {
        Ok(self
            .inventory_nodes_request(None, None, cancellation)
            .await?
            .map(|page| page.items))
    }

    /// Request an authorized DDS node page (1-100 records). Pass the opaque
    /// cursor unchanged. Older DDS returns a bounded legacy complete response
    /// on the first request, identified by `paginated == false`.
    pub async fn inventory_nodes_page(
        &self,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Option<InventoryPage<InventoryNode>>> {
        self.inventory_nodes_request(Some(limit), cursor, cancellation)
            .await
    }

    async fn inventory_nodes_request(
        &self,
        limit: Option<usize>,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Option<InventoryPage<InventoryNode>>> {
        let mut url = self.inner.client.dds_url("api/v1/nodes");
        url.query_pairs_mut()
            .append_pair("org", "all")
            .append_pair("staking_status", "all");
        inventory_page_query(&mut url, limit, cursor)?;
        #[derive(Deserialize)]
        struct Nodes {
            nodes: Vec<InventoryNode>,
            pagination: Option<InventoryPagination>,
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
        inventory_page(response.nodes, response.pagination, limit, cursor).map(Some)
    }

    /// Legacy complete assignment inventory, bounded by AuthLimits. Listing
    /// robots does not grant task authority. None means an unsupported grant.
    pub async fn inventory_robots(
        &self,
        domain_id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<InventoryRobot>>> {
        Ok(self
            .inventory_robots_request(domain_id, None, None, cancellation)
            .await?
            .map(|page| page.items))
    }

    /// Request a DDS Domain-robot page (1-100 records). Authorization and imported
    /// session allowlists are checked on every page using the shared session.
    pub async fn inventory_robots_page(
        &self,
        domain_id: Uuid,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Option<InventoryPage<InventoryRobot>>> {
        self.inventory_robots_request(domain_id, Some(limit), cursor, cancellation)
            .await
    }

    async fn inventory_robots_request(
        &self,
        domain_id: Uuid,
        limit: Option<usize>,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Option<InventoryPage<InventoryRobot>>> {
        if domain_id.is_nil() {
            return Err(Error::InvalidInput {
                field: "Domain ID",
                reason: "expected non-nil UUID",
            });
        }
        let mut url = self
            .inner
            .client
            .dds_url(&format!("api/v1/domains/{domain_id}/robots"));
        inventory_page_query(&mut url, limit, cursor)?;
        #[derive(Deserialize)]
        struct Robots {
            robots: Vec<InventoryRobot>,
            pagination: Option<InventoryPagination>,
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
        inventory_page(response.robots, response.pagination, limit, cursor).map(Some)
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
