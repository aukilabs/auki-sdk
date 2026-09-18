//! Permission-aware DDS selection; metadata visibility is distinct from data access.
use super::*;

const DISCOVERY: &str = "DDS permission-aware Domain discovery";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum DomainPermission {
    #[serde(rename = "domain-data:r")]
    DataRead,
    #[serde(rename = "domain-data:w")]
    DataWrite,
    #[serde(rename = "domain-data:d")]
    DataDelete,
    #[serde(rename = "pose:r")]
    PoseRead,
    #[serde(rename = "pose:w")]
    PoseWrite,
    #[serde(rename = "pose:d")]
    PoseDelete,
}

impl DomainPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DataRead => "domain-data:r",
            Self::DataWrite => "domain-data:w",
            Self::DataDelete => "domain-data:d",
            Self::PoseRead => "pose:r",
            Self::PoseWrite => "pose:w",
            Self::PoseDelete => "pose:d",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DomainDiscoveryQuery {
    pub organization: String,
    /// Maximum DDS candidates examined (1..=100), before permission filtering.
    pub limit: usize,
    pub cursor: Option<String>,
    /// Every requested permission must be present in the effective preview.
    pub allows: Vec<DomainPermission>,
}

impl Default for DomainDiscoveryQuery {
    fn default() -> Self {
        Self {
            organization: "own".into(),
            limit: 50,
            cursor: None,
            allows: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveredDomain {
    #[serde(flatten)]
    pub domain: DomainSummary,
    pub permissions: Vec<DomainPermission>,
}

/// A real DDS cursor page. Empty pages may have a continuation after filtering.
/// Previews are advisory; authorization is rechecked when requesting a grant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainDiscoveryPage {
    pub domains: Vec<DiscoveredDomain>,
    pub next_cursor: Option<String>,
}

impl AuthSession {
    /// Requires the permission-discovery provider contract; never falls back to
    /// ordinary visibility or mints a Domain token for picker entries.
    pub async fn discover_domains(
        &self,
        query: &DomainDiscoveryQuery,
        cancellation: &CancellationToken,
    ) -> Result<DomainDiscoveryPage> {
        if !(1..=100).contains(&query.limit)
            || !(matches!(query.organization.as_str(), "own" | "all")
                || Uuid::parse_str(&query.organization).is_ok())
            || query
                .cursor
                .as_ref()
                .is_some_and(|c| c.is_empty() || c.len() > 2048)
        {
            return Err(Error::InvalidInput {
                field: "Domain discovery query",
                reason: "expected limit 1-100, a valid organization, and a bounded opaque cursor",
            });
        }
        let operation = async {
            let mut state = self.lock_state(cancellation, DISCOVERY).await?;
            let imported = matches!(state.principal, PrincipalState::Zitadel(_));
            if imported && query.organization == "all" {
                return Err(Error::InvalidConfiguration(
                    "imported Domain discovery is limited to the user's organization",
                ));
            }
            let path = if imported {
                "api/v1/domain-discovery/zitadel"
            } else {
                "api/v1/domain-discovery"
            };
            let mut url = self.inner.client.dds_url(path);
            url.query_pairs_mut()
                .append_pair("org", &query.organization)
                .append_pair("limit", &query.limit.to_string());
            if let Some(cursor) = &query.cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            if !query.allows.is_empty() {
                let mut permissions: Vec<_> = query.allows.iter().map(|p| p.as_str()).collect();
                permissions.sort_unstable();
                permissions.dedup();
                url.query_pairs_mut()
                    .append_pair("allows", &permissions.join(","));
            }
            #[derive(Deserialize)]
            struct Pagination {
                version: u32,
                limit: usize,
                next_cursor: String,
            }
            #[derive(Deserialize)]
            struct Response {
                domains: Vec<DiscoveredDomain>,
                pagination: Pagination,
            }
            let response: Response = if let PrincipalState::Zitadel(session) = &state.principal {
                self.human_dds_json(session, url, false, DISCOVERY, cancellation)
                    .await?
            } else {
                self.data_dds_json(&mut state, url, false, DISCOVERY, cancellation)
                    .await?
            };
            let mut ids = HashSet::new();
            if response.pagination.version != 1
                || response.pagination.limit != query.limit
                || response.domains.len() > query.limit
                || response.pagination.next_cursor.len() > 2048
                || (!response.pagination.next_cursor.is_empty()
                    && query.cursor.as_deref() == Some(&response.pagination.next_cursor))
                || response.domains.iter().any(|d| {
                    d.domain.id.is_nil()
                        || !ids.insert(d.domain.id)
                        || d.domain.organization_id.is_none_or(|org| org.is_nil())
                        || query
                            .allows
                            .iter()
                            .any(|permission| !d.permissions.contains(permission))
                        || d.permissions.iter().collect::<HashSet<_>>().len() != d.permissions.len()
                })
            {
                return Err(Error::invalid_response(
                    DISCOVERY,
                    "inconsistent permission page",
                ));
            }
            Ok(DomainDiscoveryPage {
                domains: response.domains,
                next_cursor: (!response.pagination.next_cursor.is_empty())
                    .then_some(response.pagination.next_cursor),
            })
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            _ = cancellation.cancelled() => Err(Error::Cancelled { endpoint: DISCOVERY }),
            result = operation => result,
        }
    }

    // DDS returns 401 only for invalid/expired identity; policy denial is 403 and
    // ambiguous policy authentication failures are 503. Only 401 may rotate OAuth.
    pub(super) async fn human_dds_json<T: DeserializeOwned>(
        &self,
        session: &Arc<ZitadelSession>,
        url: Url,
        post: bool,
        endpoint: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        let mut ready = session.ready(RefreshMode::IfExpiring, cancellation).await?;
        for attempt in 0..2 {
            let http = &self.inner.client.inner.http;
            let request = if post {
                http.post(url.clone())
            } else {
                http.get(url.clone())
            };
            let request = request
                .bearer_auth(ready.credentials.access_token().expose())
                .header(ACCEPT, "application/json")
                .header("posemesh-client-id", self.client_id())
                .header(
                    "posemesh-sdk-version",
                    concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                );
            match self
                .inner
                .client
                .send_json(request, endpoint, cancellation)
                .await
            {
                Err(error) if error.is_unauthorized() => {
                    if attempt > 0 || ready.refreshed {
                        session.require_login().await;
                        return Err(Error::AuthenticationRequired);
                    }
                    ready = session.ready(RefreshMode::Force, cancellation).await?;
                }
                result => return result,
            }
        }
        unreachable!("second attempt always returns")
    }
}
