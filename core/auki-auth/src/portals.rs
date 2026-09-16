//! DDS portal metadata. The service still calls portals `lighthouses` on the wire.
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
            #[derive(Deserialize)]
            struct Response {
                domains: Vec<PortalDomain>,
            }
            let response: Response = self
                .data_dds_json(&mut state, url, false, DDS_PORTALS, cancellation)
                .await?;
            Ok(response.domains)
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
        let response: Response = self.portal_request(domain, None, cancellation).await?;
        Ok(response.lighthouses)
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
            .portal_request(domain, Some(portal), cancellation)
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
        cancellation: &CancellationToken,
    ) -> Result<T> {
        let operation = async {
            let mut access = self.domain_access(domain, None, cancellation).await?;
            let suffix = portal
                .map(|id| format!("/{}", id.as_str()))
                .unwrap_or_default();
            let url = self
                .inner
                .client
                .dds_url(&format!("api/v1/domains/{domain}/lighthouses{suffix}"));
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
