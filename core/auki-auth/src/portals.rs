//! DDS portal metadata. The service still calls portals `lighthouses` on the wire.
use super::super::inventory::{
    InventoryPage, InventoryPagination, inventory_page, inventory_page_query,
};
use super::*;

const DDS_PORTALS: &str = "DDS portal metadata";

/// A UUID or the service's eleven-character short ID. Validated before routing.
#[derive(Clone, Debug)]
pub struct PortalId(String);

impl PortalId {
    pub fn parse(value: &str) -> Result<Self> {
        if let Ok(id) = Uuid::parse_str(value) {
            return Ok(Self(id.to_string()));
        }
        if value.len() == 11 && value.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Ok(Self(value.to_ascii_uppercase()));
        }
        Err(Error::InvalidInput {
            field: "portal ID",
            reason: "expected a UUID or eleven alphanumeric characters",
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, id: Uuid, short_id: &str) -> bool {
        self.0 == id.to_string() || self.0.eq_ignore_ascii_case(short_id)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Portal {
    pub id: Uuid,
    pub short_id: String,
    pub name: String,
    /// The service's recorded size, in centimetres; no coordinate conversion.
    pub size: f64,
    pub organization_id: Option<Uuid>,
    pub default_domain_id: Option<Uuid>,
    pub redirect_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PortalDomain {
    #[serde(flatten)]
    pub domain: DomainSummary,
    pub is_default: bool,
    pub added_to_domain_at: DateTime<Utc>,
}

impl AuthSession {
    /// Associated Domains, without minting a token for every result. No pagination.
    pub async fn domains_for_portal(
        &self,
        portal: &PortalId,
        organization: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<PortalDomain>> {
        Ok(self
            .domains_for_portal_request(portal, organization, None, None, cancellation)
            .await?
            .items)
    }

    /// Server cursor page of portal associations. `paginated == false` reports
    /// an older provider returning a bounded complete response on the first call.
    pub async fn domains_for_portal_page(
        &self,
        portal: &PortalId,
        organization: &str,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<InventoryPage<PortalDomain>> {
        self.domains_for_portal_request(portal, organization, Some(limit), cursor, cancellation)
            .await
    }

    async fn domains_for_portal_request(
        &self,
        portal: &PortalId,
        organization: &str,
        limit: Option<usize>,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<InventoryPage<PortalDomain>> {
        if !(matches!(organization, "own" | "all") || Uuid::parse_str(organization).is_ok()) {
            return Err(Error::InvalidInput {
                field: "organization",
                reason: "expected own, all or an organization UUID",
            });
        }
        let operation = async {
            let mut state = self.lock_state(cancellation, DDS_PORTALS).await?;
            self.require_domain_listing(&state)?;
            let mut url = self
                .inner
                .client
                .dds_url(&format!("api/v1/lighthouses/{}/domains", portal.as_str()));
            url.query_pairs_mut()
                .append_pair("org", organization)
                .append_pair("issue_token", "false");
            inventory_page_query(&mut url, limit, cursor)?;
            #[derive(Deserialize)]
            struct Response {
                domains: Vec<PortalDomain>,
                pagination: Option<InventoryPagination>,
            }
            let response: Response = self
                .data_dds_json(&mut state, url, false, DDS_PORTALS, cancellation)
                .await?;
            let mut ids = HashSet::new();
            if response
                .domains
                .iter()
                .any(|d| d.domain.id.is_nil() || !ids.insert(d.domain.id))
            {
                return Err(Error::invalid_response(
                    DDS_PORTALS,
                    "invalid or duplicate Domain identity",
                ));
            }
            inventory_page(response.domains, response.pagination, limit, cursor)
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            result = operation => result,
        }
    }

    /// Portal records use a DDS-audience Domain grant and pose read permission.
    pub async fn list_portals(
        &self,
        domain: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Portal>> {
        #[derive(Deserialize)]
        struct Response {
            lighthouses: Vec<Portal>,
        }
        let response: Response = self
            .portal_request(domain, None, None, cancellation)
            .await?;
        Ok(response.lighthouses)
    }

    pub async fn list_portals_page(
        &self,
        domain: Uuid,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<InventoryPage<Portal>> {
        #[derive(Deserialize)]
        struct Response {
            lighthouses: Vec<Portal>,
            pagination: Option<InventoryPagination>,
        }
        let response: Response = self
            .portal_request(domain, None, Some((limit, cursor)), cancellation)
            .await?;
        let mut ids = HashSet::new();
        if response
            .lighthouses
            .iter()
            .any(|p| p.id.is_nil() || !ids.insert(p.id))
        {
            return Err(Error::invalid_response(
                DDS_PORTALS,
                "invalid or duplicate portal identity",
            ));
        }
        inventory_page(
            response.lighthouses,
            response.pagination,
            Some(limit),
            cursor,
        )
    }

    pub async fn get_portal(
        &self,
        domain: Uuid,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> Result<Portal> {
        #[derive(Deserialize)]
        struct Response {
            domain_id: Uuid,
            #[serde(flatten)]
            portal: Portal,
        }
        let response: Response = self
            .portal_request(domain, Some(portal), None, cancellation)
            .await?;
        if response.domain_id != domain
            || !portal.matches(response.portal.id, &response.portal.short_id)
        {
            return Err(Error::invalid_response(
                DDS_PORTALS,
                "wrong Domain or portal",
            ));
        }
        Ok(response.portal)
    }

    async fn portal_request<T: DeserializeOwned>(
        &self,
        domain: Uuid,
        portal: Option<&PortalId>,
        page: Option<(usize, Option<&str>)>,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        let operation = async {
            let suffix = portal
                .map(|id| format!("/{}", id.as_str()))
                .unwrap_or_default();
            let mut url = self
                .inner
                .client
                .dds_url(&format!("api/v1/domains/{domain}/lighthouses{suffix}"));
            if let Some((limit, cursor)) = page {
                inventory_page_query(&mut url, Some(limit), cursor)?;
            }
            let mut access = self.domain_access(domain, None, cancellation).await?;
            for attempt in 0..2 {
                if !access.dds_audience {
                    return Err(Error::invalid_response(
                        DDS_PORTALS,
                        "Domain grant has no DDS audience",
                    ));
                }
                let request = self
                    .inner
                    .client
                    .inner
                    .http
                    .get(url.clone())
                    .bearer_auth(access.bearer().expose_secret())
                    .header(ACCEPT, "application/json")
                    .header("posemesh-client-id", self.client_id())
                    .header(
                        "posemesh-sdk-version",
                        concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                    );
                match self
                    .inner
                    .client
                    .send_json(request, DDS_PORTALS, cancellation)
                    .await
                {
                    Err(error) if attempt == 0 && error.is_unauthorized() => {
                        access = self
                            .domain_access(domain, Some(&access), cancellation)
                            .await?;
                    }
                    result => return result,
                }
            }
            unreachable!("second attempt returns")
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            result = operation => result,
        }
    }
}
