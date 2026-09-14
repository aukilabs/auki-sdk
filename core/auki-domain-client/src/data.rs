use std::{future::Future, sync::Arc, time::Duration};

use auki_auth::{DomainAccess, DomainAccessProvider};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{DataError, DataLimits, DataListQuery, DataMetadata, DataWrite};

#[derive(Clone)]
pub struct AukiDomainData {
    pub(crate) provider: Arc<dyn DomainAccessProvider>,
    pub(crate) http: Client,
    pub(crate) limits: DataLimits,
}

impl AukiDomainData {
    pub fn new(credential: impl DomainAccessProvider + 'static) -> Result<Self, DataError> {
        Self::with_limits(credential, DataLimits::default())
    }

    pub fn with_limits(
        credential: impl DomainAccessProvider + 'static,
        limits: DataLimits,
    ) -> Result<Self, DataError> {
        if limits.request_timeout.is_zero()
            || limits.request_timeout > Duration::from_secs(300)
            || !(1..=4 * 1024 * 1024).contains(&limits.max_metadata_bytes)
            || !(1..=64 * 1024 * 1024).contains(&limits.max_data_bytes)
        {
            return Err(DataError::InvalidInput("invalid timeout or byte limits"));
        }
        let builder = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5));
        let http = builder
            .build()
            .map_err(|_| DataError::InvalidInput("cannot construct HTTP client"))?;
        Ok(Self {
            provider: Arc::new(credential),
            http,
            limits,
        })
    }

    /// Select a Domain without making a request or obtaining a token.
    pub fn in_domain(&self, domain_id: Uuid) -> DomainDataClient {
        DomainDataClient {
            client: self.clone(),
            domain_id,
            lifetime: Arc::new(Lifetime {
                closed: CancellationToken::new(),
                active: RwLock::new(()),
            }),
        }
    }
}

pub(crate) struct Lifetime {
    pub(crate) closed: CancellationToken,
    pub(crate) active: RwLock<()>,
}

/// A selected Domain. Clones share this client's close state; separately created
/// clients and peers retain their shared credential until the host closes it.
#[derive(Clone)]
pub struct DomainDataClient {
    pub(crate) client: AukiDomainData,
    pub(crate) domain_id: Uuid,
    pub(crate) lifetime: Arc<Lifetime>,
}

impl DomainDataClient {
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Cancel and drain this client's in-flight requests. Does not log out.
    pub async fn close(&self) {
        self.lifetime.closed.cancel();
        let _drained = self.lifetime.active.write().await;
    }

    pub async fn list(&self, query: &DataListQuery) -> Result<Vec<DataMetadata>, DataError> {
        self.list_with_cancellation(query, &CancellationToken::new())
            .await
    }

    pub async fn list_with_cancellation(
        &self,
        query: &DataListQuery,
        cancellation: &CancellationToken,
    ) -> Result<Vec<DataMetadata>, DataError> {
        if query.ids.len() > 100 {
            return Err(DataError::InvalidInput("at most 100 data IDs per query"));
        }
        for value in [&query.name, &query.data_type].into_iter().flatten() {
            validate_text(value)?;
        }
        let result: MetadataList = self
            .json_request(
                |http, access| {
                    let mut url = self.data_url(access, None);
                    if let Some(name) = &query.name {
                        url.query_pairs_mut().append_pair("name", name);
                    }
                    if let Some(kind) = &query.data_type {
                        url.query_pairs_mut().append_pair("data_type", kind);
                    }
                    if !query.ids.is_empty() {
                        url.query_pairs_mut().append_pair(
                            "ids",
                            &query
                                .ids
                                .iter()
                                .map(Uuid::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                    http.get(url)
                },
                cancellation,
            )
            .await?;
        for metadata in &result.data {
            self.validate_metadata(metadata, None)?;
        }
        Ok(result.data)
    }

    pub async fn get(&self, id: Uuid) -> Result<DataMetadata, DataError> {
        self.get_with_cancellation(id, &CancellationToken::new())
            .await
    }

    pub async fn get_with_cancellation(
        &self,
        id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<DataMetadata, DataError> {
        let metadata = self
            .json_request(
                |http, access| http.get(self.data_url(access, Some(id))),
                cancellation,
            )
            .await?;
        self.validate_metadata(&metadata, Some(id))?;
        Ok(metadata)
    }

    /// Read bounded bytes. Oversized data fails; it is never silently truncated.
    pub async fn read(&self, id: Uuid) -> Result<Vec<u8>, DataError> {
        self.read_with_cancellation(id, &CancellationToken::new())
            .await
    }

    pub async fn read_with_cancellation(
        &self,
        id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, DataError> {
        self.run(
            cancellation,
            self.request(
                |http, access| {
                    http.get(self.data_url(access, Some(id)))
                        .query(&[("raw", "true")])
                },
                self.client.limits.max_data_bytes,
                false,
                cancellation,
            ),
        )
        .await
    }

    pub async fn write(
        &self,
        target: DataWrite<'_>,
        bytes: &[u8],
    ) -> Result<DataMetadata, DataError> {
        self.write_with_cancellation(target, bytes, &CancellationToken::new())
            .await
    }

    pub async fn write_with_cancellation(
        &self,
        target: DataWrite<'_>,
        bytes: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<DataMetadata, DataError> {
        if bytes.len() > self.client.limits.max_data_bytes {
            return Err(DataError::TooLarge {
                maximum: self.client.limits.max_data_bytes,
            });
        }
        let (body, content_type) = multipart(target, bytes)?;
        self.run(cancellation, async {
            let response = self
                .request_checked(
                    |http, access| {
                        let url = self.data_url(access, None);
                        let request = match target {
                            DataWrite::Named { .. } => http.post(url),
                            DataWrite::ById(_) => http.put(url),
                        };
                        request
                            .header("Content-Type", &content_type)
                            .body(body.clone())
                    },
                    self.client.limits.max_metadata_bytes,
                    true,
                    cancellation,
                    Some((bytes.len(), body.len())),
                )
                .await?;
            let result: MetadataList = decode(&response)?;
            if result.data.len() != 1 {
                return Err(DataError::InvalidResponse("expected one stored item"));
            }
            let metadata = result.data.into_iter().next().expect("checked one item");
            self.validate_metadata(
                &metadata,
                match target {
                    DataWrite::ById(id) => Some(id),
                    _ => None,
                },
            )?;
            Ok(metadata)
        })
        .await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), DataError> {
        self.delete_with_cancellation(id, &CancellationToken::new())
            .await
    }

    pub async fn delete_with_cancellation(
        &self,
        id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<(), DataError> {
        self.run(
            cancellation,
            self.request(
                |http, access| http.delete(self.data_url(access, Some(id))),
                self.client.limits.max_metadata_bytes,
                false,
                cancellation,
            ),
        )
        .await?;
        Ok(())
    }

    pub(crate) fn data_url(&self, access: &DomainAccess, id: Option<Uuid>) -> reqwest::Url {
        let path = format!("api/v1/domains/{}/data", self.domain_id);
        access
            .server_url()
            .join(&match id {
                Some(id) => format!("{path}/{id}"),
                None => path,
            })
            .expect("UUID path")
    }

    pub(crate) fn validate_metadata(
        &self,
        metadata: &DataMetadata,
        id: Option<Uuid>,
    ) -> Result<(), DataError> {
        if metadata.domain_id != self.domain_id || id.is_some_and(|id| id != metadata.id) {
            return Err(DataError::InvalidResponse(
                "metadata belongs to another Domain or item",
            ));
        }
        Ok(())
    }

    pub(crate) async fn json_request<T: DeserializeOwned>(
        &self,
        make: impl Fn(&Client, &DomainAccess) -> RequestBuilder,
        cancellation: &CancellationToken,
    ) -> Result<T, DataError> {
        let bytes = self
            .run(
                cancellation,
                self.request(
                    make,
                    self.client.limits.max_metadata_bytes,
                    true,
                    cancellation,
                ),
            )
            .await?;
        decode(&bytes)
    }

    async fn request(
        &self,
        make: impl Fn(&Client, &DomainAccess) -> RequestBuilder,
        maximum: usize,
        json: bool,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, DataError> {
        self.request_checked(make, maximum, json, cancellation, None)
            .await
    }

    async fn request_checked(
        &self,
        make: impl Fn(&Client, &DomainAccess) -> RequestBuilder,
        maximum: usize,
        json: bool,
        cancellation: &CancellationToken,
        upload_sizes: Option<(usize, usize)>,
    ) -> Result<Vec<u8>, DataError> {
        let mut access = self
            .client
            .provider
            .domain_access(self.domain_id, None, cancellation)
            .await?;
        for attempt in 0..2 {
            if access.domain_id() != self.domain_id || access.expires_at() <= chrono::Utc::now() {
                return Err(DataError::InvalidResponse(
                    "wrong-Domain or expired data grant",
                ));
            }
            if let Some((file_size, request_size)) = upload_sizes {
                // Check the same server on every attempt, including after DDS
                // rotates a grant to a different Domain Server. Info is public.
                let request = self
                    .client
                    .http
                    .get(
                        access
                            .server_url()
                            .join("api/v1/info")
                            .expect("static path"),
                    )
                    .header("Accept", "application/json")
                    .header("posemesh-client-id", self.client.provider.client_id())
                    .header(
                        "posemesh-sdk-version",
                        concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                    );
                let bytes =
                    crate::http::send(request, self.client.limits.max_metadata_bytes, true).await?;
                let info: ServerInfo = decode(&bytes)?;
                for (size, limit) in [
                    (file_size, info.upload.domain_data_max_bytes),
                    (request_size, info.upload.request_max_bytes),
                ] {
                    if limit < 0 {
                        return Err(DataError::InvalidResponse("negative server upload limit"));
                    }
                    if limit > 0 && size as u64 > limit as u64 {
                        return Err(DataError::TooLarge {
                            maximum: usize::try_from(limit).unwrap_or(usize::MAX),
                        });
                    }
                }
            }
            let request = make(&self.client.http, &access)
                .bearer_auth(access.bearer().expose_secret())
                .header("posemesh-client-id", self.client.provider.client_id())
                .header(
                    "posemesh-sdk-version",
                    concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                )
                .header(
                    "Accept",
                    if json {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    },
                );
            match crate::http::send(request, maximum, json).await {
                Err(DataError::HttpStatus { status: 401 }) if attempt == 0 => {
                    access = self
                        .client
                        .provider
                        .domain_access(self.domain_id, Some(&access), cancellation)
                        .await?;
                }
                result => return result,
            }
        }
        unreachable!("second attempt returns")
    }

    pub(crate) async fn run<T>(
        &self,
        cancellation: &CancellationToken,
        operation: impl Future<Output = Result<T, DataError>>,
    ) -> Result<T, DataError> {
        let _active = self.lifetime.active.read().await;
        self.run_transfer(cancellation, operation).await
    }

    pub(crate) async fn run_transfer<T>(
        &self,
        cancellation: &CancellationToken,
        operation: impl Future<Output = Result<T, DataError>>,
    ) -> Result<T, DataError> {
        tokio::select! {
            biased;
            _ = self.lifetime.closed.cancelled() => Err(DataError::Closed),
            _ = self.client.provider.wait_closed() => Err(DataError::Auth(auki_auth::Error::SessionClosed)),
            _ = cancellation.cancelled() => Err(DataError::Cancelled),
            _ = futures_timer::Delay::new(self.client.limits.request_timeout) => Err(DataError::TimedOut),
            result = operation => result,
        }
    }
}

#[derive(Deserialize)]
struct MetadataList {
    data: Vec<DataMetadata>,
}
#[derive(Deserialize)]
struct ServerInfo {
    upload: UploadLimits,
}
#[derive(Deserialize)]
struct UploadLimits {
    domain_data_max_bytes: i64,
    request_max_bytes: i64,
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DataError> {
    serde_json::from_slice(bytes)
        .map_err(|_| DataError::InvalidResponse("unexpected JSON contract"))
}

fn validate_text(value: &str) -> Result<(), DataError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(DataError::InvalidInput(
            "names and data types require 1-128 bytes without control characters",
        ));
    }
    Ok(())
}

// Preserve the Posemesh/Domain Server multipart parameters, escaping quoted
// values and choosing a fresh boundary absent from the payload.
pub(crate) fn multipart(
    target: DataWrite<'_>,
    bytes: &[u8],
) -> Result<(Vec<u8>, String), DataError> {
    let disposition = match target {
        DataWrite::Named { name, data_type } => {
            validate_text(name)?;
            validate_text(data_type)?;
            // domain-service/pkg/utils/disallowedchars.go applies to both fields.
            if [name, data_type]
                .iter()
                .any(|value| value.contains(|c| ";!?<>[]{}()/\\\"$#@*^&|~%=+".contains(c)))
            {
                return Err(DataError::InvalidInput(
                    "name or data type contains server-disallowed punctuation",
                ));
            }
            let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            format!(
                "name=\"{}\"; data-type=\"{}\"",
                escape(name),
                escape(data_type)
            )
        }
        DataWrite::ById(id) => format!("id=\"{id}\""),
    };
    let boundary = loop {
        let boundary = format!("auki-{}", Uuid::new_v4());
        if !bytes
            .windows(boundary.len())
            .any(|window| window == boundary.as_bytes())
        {
            break boundary;
        }
    };
    let mut body = format!("--{boundary}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: form-data; {disposition}\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((body, format!("multipart/form-data; boundary={boundary}")))
}
