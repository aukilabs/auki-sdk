//! Pull/push callbacks keep ownership and backpressure with the calling application.
use crate::data::{decode, multipart};
use crate::{DataError, DataMetadata, DataWrite, DomainDataClient, TransferOptions};
use auki_auth::DomainAccess;
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl TransferOptions {
    fn validate(self) -> Result<(), DataError> {
        if self.max_bytes == 0
            || self.max_bytes > i64::MAX as u64
            || !(1..=64 * 1024 * 1024).contains(&self.max_chunk_bytes)
        {
            return Err(DataError::InvalidInput("invalid streaming byte limits"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Info {
    upload: UploadInfo,
}
#[derive(Deserialize)]
struct UploadInfo {
    request_max_bytes: i64,
    domain_data_max_bytes: i64,
    multipart: MultipartInfo,
}
#[derive(Deserialize)]
struct MultipartInfo {
    enabled: bool,
    part_size_bytes: i64,
}
#[derive(Deserialize)]
struct Initiated {
    upload_id: Uuid,
    data_id: Uuid,
    part_size: i64,
    expires_at: DateTime<Utc>,
}
#[derive(Deserialize)]
struct PartResponse {
    etag: String,
}
#[derive(Serialize)]
struct Part {
    part_number: u32,
    etag: String,
}

impl DomainDataClient {
    /// Stream a single item's raw bytes to an awaited destination. The callback is
    /// never called concurrently. Each request/read/callback has the client timeout.
    /// Cancellation or failure can leave a partial destination; no bytes are replayed.
    pub async fn read_to<F, Fut>(
        &self,
        id: Uuid,
        options: TransferOptions,
        cancellation: &CancellationToken,
        mut write_chunk: F,
    ) -> Result<u64, DataError>
    where
        F: FnMut(Vec<u8>) -> Fut,
        Fut: Future<Output = Result<(), DataError>>,
    {
        options.validate()?;
        let _active = self.lifetime.active.read().await;
        let mut access = self
            .run_transfer(cancellation, async {
                Ok(self
                    .client
                    .provider
                    .domain_access(self.domain_id, None, cancellation)
                    .await?)
            })
            .await?;
        let server = access.server_url().clone();
        let mut response = self
            .run_transfer(
                cancellation,
                self.transfer_open(
                    &mut access,
                    &server,
                    |http, grant| {
                        http.get(self.data_url(grant, Some(id)))
                            .query(&[("raw", "true")])
                    },
                    options.max_bytes,
                    false,
                    cancellation,
                ),
            )
            .await?;
        let mut total = 0;
        while let Some(chunk) = self
            .run_transfer(cancellation, response.next(options.max_chunk_bytes))
            .await?
        {
            total += chunk.len() as u64;
            self.run_transfer(cancellation, async { write_chunk(chunk).await })
                .await?;
        }
        Ok(total)
    }

    /// Upload a known-size stream using the server's multipart session contract.
    /// `read_chunk(maximum)` returns at most that many bytes; empty means EOF.
    /// Named multipart completion may replace an existing name. ById preserves ID.
    /// Cancel via the token and await this future to abort known upload sessions.
    /// An abandoned future/process or lost initiation response relies on server TTL.
    pub async fn write_stream<F, Fut>(
        &self,
        target: DataWrite<'_>,
        size: u64,
        options: TransferOptions,
        cancellation: &CancellationToken,
        mut read_chunk: F,
    ) -> Result<DataMetadata, DataError>
    where
        F: FnMut(usize) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, DataError>>,
    {
        self.require_write_access()?;
        options.validate()?;
        if size == 0 || size > options.max_bytes {
            return Err(DataError::InvalidInput(
                "stream size must be nonzero and within max_bytes",
            ));
        }
        // Share exactly the buffered writer's name/type validation, without file bytes.
        multipart(target, &[])?;
        let _active = self.lifetime.active.read().await;
        let mut access = self
            .run_transfer(cancellation, async {
                Ok(self
                    .client
                    .provider
                    .domain_access(self.domain_id, None, cancellation)
                    .await?)
            })
            .await?;
        let server = access.server_url().clone();
        let info: Info = self
            .run_transfer(cancellation, async {
                let request = self
                    .client
                    .http
                    .get(server.join("api/v1/info").expect("static path"))
                    .header("Accept", "application/json")
                    .header("posemesh-client-id", self.client.provider.client_id())
                    .header(
                        "posemesh-sdk-version",
                        concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                    );
                decode(
                    &crate::http::send(request, self.client.limits.max_metadata_bytes, true)
                        .await?,
                )
            })
            .await?;
        if !info.upload.multipart.enabled {
            return Err(DataError::InvalidInput(
                "selected server does not enable multipart uploads",
            ));
        }
        check_limit(size, info.upload.domain_data_max_bytes)?;
        check_part_size(
            info.upload.multipart.part_size_bytes,
            info.upload.request_max_bytes,
            options,
        )?;
        let payload = match target {
            DataWrite::Named { name, data_type } => {
                serde_json::json!({"name": name, "data_type": data_type, "size": size, "content_type":"application/octet-stream"})
            }
            DataWrite::ById(id) => {
                serde_json::json!({"existing_id": id, "size": size, "content_type":"application/octet-stream"})
            }
        };
        let body = serde_json::to_vec(&payload)
            .map_err(|_| DataError::InvalidInput("invalid upload metadata"))?;
        check_limit(body.len() as u64, info.upload.request_max_bytes)?;
        let initiated: Initiated = self
            .run_transfer(cancellation, async {
                let mut response = self
                    .transfer_open(
                        &mut access,
                        &server,
                        |http, grant| {
                            http.post(self.multipart_url(grant, None))
                                .query(&[("uploads", "")])
                                .header("Content-Type", "application/json")
                                .body(body.clone())
                        },
                        self.client.limits.max_metadata_bytes as u64,
                        true,
                        cancellation,
                    )
                    .await?;
                decode(&collect(&mut response).await?)
            })
            .await?;
        let result = async {
            let part_size =
                check_part_size(initiated.part_size, info.upload.request_max_bytes, options)?;
            if size.div_ceil(part_size as u64) > 10_000 {
                return Err(DataError::InvalidInput(
                    "multipart upload exceeds 10000 parts",
                ));
            }
            if let DataWrite::ById(id) = target
                && id != initiated.data_id
            {
                return Err(DataError::InvalidResponse(
                    "multipart upload targets another data ID",
                ));
            }
            let mut remaining = size;
            let mut parts = Vec::new();
            while remaining > 0 {
                check_expiry(initiated.expires_at)?;
                let needed = remaining.min(part_size as u64) as usize;
                let mut buffer = Vec::with_capacity(needed);
                while buffer.len() < needed {
                    let maximum = needed - buffer.len();
                    let bytes = self
                        .run_transfer(cancellation, async { read_chunk(maximum).await })
                        .await?;
                    if bytes.is_empty() || bytes.len() > maximum {
                        return Err(DataError::InvalidInput(
                            "source ended early or exceeded requested chunk size",
                        ));
                    }
                    buffer.extend_from_slice(&bytes);
                }
                let part_number = parts.len() as u32 + 1;
                let bytes = bytes::Bytes::from(buffer);
                let response: PartResponse = self
                    .run_transfer(cancellation, async {
                        let mut response = self
                            .transfer_open(
                                &mut access,
                                &server,
                                |http, grant| {
                                    http.put(self.multipart_url(grant, Some(initiated.upload_id)))
                                        .query(&[("partNumber", part_number)])
                                        .header("Content-Type", "application/octet-stream")
                                        .body(bytes.clone())
                                },
                                self.client.limits.max_metadata_bytes as u64,
                                true,
                                cancellation,
                            )
                            .await?;
                        decode(&collect(&mut response).await?)
                    })
                    .await?;
                if response.etag.is_empty()
                    || response.etag.len() > 1024
                    || response.etag.chars().any(char::is_control)
                {
                    return Err(DataError::InvalidResponse("invalid multipart ETag"));
                }
                parts.push(Part {
                    part_number,
                    etag: response.etag,
                });
                remaining -= needed as u64;
            }
            if !self
                .run_transfer(cancellation, async { read_chunk(1).await })
                .await?
                .is_empty()
            {
                return Err(DataError::InvalidInput("source exceeded declared size"));
            }
            check_expiry(initiated.expires_at)?;
            let body = serde_json::to_vec(&serde_json::json!({"parts": parts}))
                .map_err(|_| DataError::InvalidInput("invalid completion metadata"))?;
            check_limit(body.len() as u64, info.upload.request_max_bytes)?;
            if body.len() > self.client.limits.max_metadata_bytes {
                return Err(DataError::TooLarge {
                    maximum: self.client.limits.max_metadata_bytes,
                });
            }
            let metadata: DataMetadata = self
                .run_transfer(cancellation, async {
                    let mut response = self
                        .transfer_open(
                            &mut access,
                            &server,
                            |http, grant| {
                                http.post(self.multipart_url(grant, Some(initiated.upload_id)))
                                    .header("Content-Type", "application/json")
                                    .body(body.clone())
                            },
                            self.client.limits.max_metadata_bytes as u64,
                            true,
                            cancellation,
                        )
                        .await?;
                    decode(&collect(&mut response).await?)
                })
                .await?;
            self.validate_metadata(
                &metadata,
                match target {
                    DataWrite::ById(id) => Some(id),
                    _ => None,
                },
            )?;
            if metadata.size != size {
                return Err(DataError::InvalidResponse("completed upload size mismatch"));
            }
            Ok(metadata)
        }
        .await;
        if let Err(operation) = result {
            // Cleanup runs even when the operation's cancellation/client-close token
            // fired. The outer active permit makes close() await this bounded attempt.
            let cleanup = self.abort_upload(&access, initiated.upload_id).await;
            return Err(match cleanup {
                Ok(()) => operation,
                Err(cleanup) => DataError::Cleanup {
                    operation: Box::new(operation),
                    cleanup: Box::new(cleanup),
                },
            });
        }
        result
    }

    fn multipart_url(&self, access: &DomainAccess, upload_id: Option<Uuid>) -> Url {
        let mut url = access
            .server_url()
            .join(&format!("api/v1/domains/{}/data/multipart", self.domain_id))
            .expect("UUID path");
        if let Some(id) = upload_id {
            url.query_pairs_mut()
                .append_pair("uploadId", &id.to_string());
        }
        url
    }

    async fn transfer_open(
        &self,
        access: &mut Arc<DomainAccess>,
        server: &Url,
        make: impl Fn(&Client, &DomainAccess) -> RequestBuilder,
        maximum: u64,
        json: bool,
        cancellation: &CancellationToken,
    ) -> Result<crate::http::ResponseBody, DataError> {
        let mut candidate = self
            .client
            .provider
            .domain_access(self.domain_id, None, cancellation)
            .await?;
        for attempt in 0..2 {
            if candidate.domain_id() != self.domain_id || candidate.expires_at() <= Utc::now() {
                return Err(DataError::InvalidResponse(
                    "wrong-Domain or expired data grant",
                ));
            }
            // Multipart IDs and partial downloads belong to one immutable server.
            if candidate.server_url() != server {
                return Err(DataError::InvalidResponse(
                    "Domain Server changed during transfer",
                ));
            }
            *access = candidate.clone();
            let request = self.authorize_transfer(make(&self.client.http, access), access, json);
            match crate::http::open(request, maximum, json).await {
                Err(DataError::HttpStatus { status: 401 }) if attempt == 0 => {
                    candidate = self
                        .client
                        .provider
                        .domain_access(self.domain_id, Some(access), cancellation)
                        .await?;
                }
                result => return result,
            }
        }
        unreachable!("second attempt returns")
    }

    fn authorize_transfer(
        &self,
        request: RequestBuilder,
        access: &DomainAccess,
        json: bool,
    ) -> RequestBuilder {
        request
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
            )
    }

    async fn abort_upload(&self, access: &Arc<DomainAccess>, id: Uuid) -> Result<(), DataError> {
        let operation = async {
            let mut current = access.clone();
            for attempt in 0..2 {
                let request = self.authorize_transfer(
                    self.client
                        .http
                        .delete(self.multipart_url(&current, Some(id))),
                    &current,
                    false,
                );
                match crate::http::send(request, self.client.limits.max_metadata_bytes, false).await
                {
                    Err(DataError::HttpStatus { status: 401 }) if attempt == 0 => {
                        current = self
                            .client
                            .provider
                            .domain_access(
                                self.domain_id,
                                Some(&current),
                                &CancellationToken::new(),
                            )
                            .await?;
                        if current.server_url() != access.server_url() {
                            return Err(DataError::InvalidResponse(
                                "Domain Server changed during cleanup",
                            ));
                        }
                    }
                    result => return result.map(|_| ()),
                }
            }
            unreachable!("second attempt returns")
        };
        tokio::select! {
            result = operation => result,
            _ = futures_timer::Delay::new(Duration::from_secs(5)) => Err(DataError::TimedOut),
        }
    }
}

async fn collect(body: &mut crate::http::ResponseBody) -> Result<Vec<u8>, DataError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next(64 * 1024).await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn check_limit(size: u64, maximum: i64) -> Result<(), DataError> {
    if maximum < 0 {
        return Err(DataError::InvalidResponse("negative server upload limit"));
    }
    if maximum > 0 && size > maximum as u64 {
        return Err(DataError::TooLarge {
            maximum: usize::try_from(maximum).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}
fn check_part_size(
    size: i64,
    request_maximum: i64,
    options: TransferOptions,
) -> Result<usize, DataError> {
    if size <= 0 {
        return Err(DataError::InvalidResponse("invalid server part size"));
    }
    if size as u64 > options.max_chunk_bytes as u64 {
        return Err(DataError::TooLarge {
            maximum: options.max_chunk_bytes,
        });
    }
    check_limit(size as u64, request_maximum)?;
    Ok(size as usize)
}
fn check_expiry(expiry: DateTime<Utc>) -> Result<(), DataError> {
    if expiry <= Utc::now() {
        return Err(DataError::InvalidResponse("multipart session expired"));
    }
    Ok(())
}
