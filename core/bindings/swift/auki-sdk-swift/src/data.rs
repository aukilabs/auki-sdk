//! Domain discovery and HTTP data bindings using the session's shared refresh owner.

use std::sync::Arc;

use auki_sdk_rs::{
    AukiDomainData as DataFactory, AukiDomains as RustDomains, DataError, DataListQuery,
    DataMetadata, DataWrite, DomainDataClient, DomainListQuery, DomainPage, DomainSummary, Portal,
    PortalDomain, PortalId, PortalPose, TransferOptions,
};
use parking_lot::Mutex;
use tokio::{
    runtime::Handle,
    sync::{Mutex as AsyncMutex, mpsc, watch},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{AukiAuthFailureKind, AukiSdkError, AukiSession, wait_cleanup};

const DEFAULT_DOMAIN_PAGE_LIMIT: u32 = 50;
const DEFAULT_TRANSFER_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_TRANSFER_MAX_CHUNK_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiDataFailureKind {
    Authentication,
    Http,
    InvalidInput,
    InvalidResponse,
    Limit,
    Cancelled,
    Closed,
    Timeout,
    Transport,
    Callback,
    Cleanup,
}

#[derive(Clone, Debug)]
struct TransferFailure {
    kind: AukiDataFailureKind,
    status: Option<u16>,
    auth_kind: Option<AukiAuthFailureKind>,
    message: String,
}

impl From<DataError> for TransferFailure {
    fn from(error: DataError) -> Self {
        let kind = match &error {
            DataError::Auth(_) => AukiDataFailureKind::Authentication,
            DataError::HttpStatus { .. } => AukiDataFailureKind::Http,
            DataError::InvalidInput(_) => AukiDataFailureKind::InvalidInput,
            DataError::InvalidResponse(_) => AukiDataFailureKind::InvalidResponse,
            DataError::TooLarge { .. } => AukiDataFailureKind::Limit,
            DataError::Cancelled => AukiDataFailureKind::Cancelled,
            DataError::Closed => AukiDataFailureKind::Closed,
            DataError::TimedOut => AukiDataFailureKind::Timeout,
            DataError::Transport => AukiDataFailureKind::Transport,
            DataError::Callback => AukiDataFailureKind::Callback,
            DataError::Cleanup { .. } => AukiDataFailureKind::Cleanup,
        };
        Self {
            kind,
            status: error.status(),
            auth_kind: match &error {
                DataError::Auth(error) => Some(error.kind().into()),
                DataError::Cleanup { operation, .. } => match operation.as_ref() {
                    DataError::Auth(error) => Some(error.kind().into()),
                    _ => None,
                },
                _ => None,
            },
            message: error.to_string(),
        }
    }
}

impl From<TransferFailure> for AukiSdkError {
    fn from(error: TransferFailure) -> Self {
        Self::DomainData {
            kind: error.kind,
            status: error.status,
            auth_kind: error.auth_kind,
            message: error.message,
        }
    }
}

fn data_error(error: DataError) -> AukiSdkError {
    TransferFailure::from(error).into()
}

fn invalid_input(message: impl Into<String>) -> AukiSdkError {
    AukiSdkError::DomainData {
        kind: AukiDataFailureKind::InvalidInput,
        status: None,
        auth_kind: None,
        message: message.into(),
    }
}

fn parse_uuid(value: &str, label: &'static str) -> Result<Uuid, AukiSdkError> {
    Uuid::parse_str(value).map_err(|_| invalid_input(format!("{label} must be a UUID")))
}

fn parse_portal(value: &str) -> Result<PortalId, AukiSdkError> {
    PortalId::parse(value).map_err(|error| data_error(error.into()))
}

fn operation_token(value: Option<Arc<AukiCancellation>>) -> CancellationToken {
    value
        .map(|value| value.token.child_token())
        .unwrap_or_default()
}

#[derive(uniffi::Object)]
pub struct AukiCancellation {
    pub(crate) token: CancellationToken,
}

#[uniffi::export]
impl AukiCancellation {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            token: CancellationToken::new(),
        })
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct AukiDomainListQuery {
    #[uniffi(default = None)]
    pub organization: Option<String>,
    #[uniffi(default = None)]
    pub domain_server_id: Option<String>,
    #[uniffi(default = None)]
    pub limit: Option<u32>,
    #[uniffi(default = None)]
    pub offset: Option<u32>,
}

impl AukiDomainListQuery {
    fn parse(self) -> Result<DomainListQuery, AukiSdkError> {
        Ok(DomainListQuery {
            organization: self.organization.unwrap_or_else(|| "own".into()),
            domain_server_id: self
                .domain_server_id
                .as_deref()
                .map(|id| parse_uuid(id, "Domain Server ID"))
                .transpose()?,
            limit: self.limit.unwrap_or(DEFAULT_DOMAIN_PAGE_LIMIT),
            offset: self.offset.unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiDomainSummary {
    pub id: String,
    pub name: String,
    pub organization_id: Option<String>,
}

impl From<DomainSummary> for AukiDomainSummary {
    fn from(value: DomainSummary) -> Self {
        Self {
            id: value.id.to_string(),
            name: value.name,
            organization_id: value.organization_id.map(|id| id.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct AukiDomainDiscoveryQuery {
    #[uniffi(default = None)]
    pub organization: Option<String>,
    #[uniffi(default = None)]
    pub limit: Option<u32>,
    #[uniffi(default = None)]
    pub cursor: Option<String>,
    #[uniffi(default = None)]
    pub allows: Option<Vec<String>>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiDiscoveredDomain {
    pub domain: AukiDomainSummary,
    pub permissions: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiDomainDiscoveryPage {
    pub domains: Vec<AukiDiscoveredDomain>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiPortalPage {
    pub items: Vec<AukiPortal>,
    pub next_cursor: Option<String>,
    pub paginated: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiPortalDomainPage {
    pub items: Vec<AukiPortalDomain>,
    pub next_cursor: Option<String>,
    pub paginated: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiDomainPage {
    pub domains: Vec<AukiDomainSummary>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

impl From<DomainPage> for AukiDomainPage {
    fn from(value: DomainPage) -> Self {
        Self {
            domains: value.domains.into_iter().map(Into::into).collect(),
            total: value.total,
            limit: value.limit,
            offset: value.offset,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiPortalDomain {
    pub id: String,
    pub name: String,
    pub organization_id: Option<String>,
    pub is_default: bool,
    pub added_to_domain_at: String,
}

impl From<PortalDomain> for AukiPortalDomain {
    fn from(value: PortalDomain) -> Self {
        Self {
            id: value.domain.id.to_string(),
            name: value.domain.name,
            organization_id: value.domain.organization_id.map(|id| id.to_string()),
            is_default: value.is_default,
            added_to_domain_at: value.added_to_domain_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiPortal {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub size: f64,
    pub organization_id: Option<String>,
    pub default_domain_id: Option<String>,
    pub redirect_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Portal> for AukiPortal {
    fn from(value: Portal) -> Self {
        Self {
            id: value.id.to_string(),
            short_id: value.short_id,
            name: value.name,
            size: value.size,
            organization_id: value.organization_id.map(|id| id.to_string()),
            default_domain_id: value.default_domain_id.map(|id| id.to_string()),
            redirect_url: value.redirect_url,
            created_at: value.created_at.to_rfc3339(),
            updated_at: value.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiPortalPose {
    pub id: String,
    pub short_id: String,
    pub domain_id: String,
    pub reported_size: f64,
    pub px: f64,
    pub py: f64,
    pub pz: f64,
    pub rx: f64,
    pub ry: f64,
    pub rz: f64,
    pub rw: f64,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub vertical_accuracy: Option<f64>,
    pub horizontal_accuracy: Option<f64>,
    pub gps_timestamp: Option<f64>,
    pub scanner_device_id: String,
    pub scanner_device_name: String,
    pub scanner_device_model: String,
    pub placed_at: String,
}

impl From<PortalPose> for AukiPortalPose {
    fn from(value: PortalPose) -> Self {
        Self {
            id: value.id.to_string(),
            short_id: value.short_id,
            domain_id: value.domain_id.to_string(),
            reported_size: value.reported_size,
            px: value.px,
            py: value.py,
            pz: value.pz,
            rx: value.rx,
            ry: value.ry,
            rz: value.rz,
            rw: value.rw,
            latitude: value.latitude,
            longitude: value.longitude,
            altitude: value.altitude,
            vertical_accuracy: value.vertical_accuracy,
            horizontal_accuracy: value.horizontal_accuracy,
            gps_timestamp: value.gps_timestamp,
            scanner_device_id: value.scanner_device_id,
            scanner_device_name: value.scanner_device_name,
            scanner_device_model: value.scanner_device_model,
            placed_at: value.placed_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct AukiDataListQuery {
    #[uniffi(default = [])]
    pub ids: Vec<String>,
    #[uniffi(default = None)]
    pub name: Option<String>,
    #[uniffi(default = None)]
    pub data_type: Option<String>,
}

impl AukiDataListQuery {
    fn parse(self) -> Result<DataListQuery, AukiSdkError> {
        Ok(DataListQuery {
            ids: self
                .ids
                .iter()
                .map(|id| parse_uuid(id, "data ID"))
                .collect::<Result<_, _>>()?,
            name: self.name,
            data_type: self.data_type,
        })
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AukiDataMetadata {
    pub id: String,
    pub domain_id: String,
    pub name: String,
    pub data_type: String,
    pub size: u64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<DataMetadata> for AukiDataMetadata {
    fn from(value: DataMetadata) -> Self {
        Self {
            id: value.id.to_string(),
            domain_id: value.domain_id.to_string(),
            name: value.name,
            data_type: value.data_type,
            size: value.size,
            created_at: value.created_at.to_rfc3339(),
            updated_at: value.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum AukiDataWriteTarget {
    Named { name: String, data_type: String },
    ById { id: String },
}

impl AukiDataWriteTarget {
    fn as_write(&self) -> Result<DataWrite<'_>, AukiSdkError> {
        match self {
            Self::Named { name, data_type } => Ok(DataWrite::Named { name, data_type }),
            Self::ById { id } => Ok(DataWrite::ById(parse_uuid(id, "data ID")?)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, uniffi::Record)]
pub struct AukiTransferOptions {
    #[uniffi(default = None)]
    pub max_bytes: Option<u64>,
    #[uniffi(default = None)]
    pub max_chunk_bytes: Option<u64>,
}

impl AukiTransferOptions {
    fn parse(self) -> Result<TransferOptions, AukiSdkError> {
        let max_chunk_bytes = self
            .max_chunk_bytes
            .unwrap_or(DEFAULT_TRANSFER_MAX_CHUNK_BYTES);
        Ok(TransferOptions {
            max_bytes: self.max_bytes.unwrap_or(DEFAULT_TRANSFER_MAX_BYTES),
            max_chunk_bytes: usize::try_from(max_chunk_bytes)
                .map_err(|_| invalid_input("maxChunkBytes exceeds this platform's range"))?,
        })
    }
}

#[derive(uniffi::Object)]
pub struct AukiDomains {
    inner: RustDomains,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDomains {
    #[uniffi::method(default(cancellation = None))]
    pub async fn discover(
        &self,
        query: AukiDomainDiscoveryQuery,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiDomainDiscoveryPage, AukiSdkError> {
        let query: auki_sdk_rs::DomainDiscoveryQuery = serde_json::from_value(serde_json::json!({
            "organization": query.organization.unwrap_or_else(|| "own".into()),
            "limit": query.limit.unwrap_or(50),
            "cursor": query.cursor,
            "allows": query.allows.unwrap_or_default(),
        }))
        .map_err(|_| invalid_input("invalid Domain discovery query"))?;
        let page = self
            .inner
            .discover(&query, &operation_token(cancellation))
            .await
            .map_err(data_error)?;
        Ok(AukiDomainDiscoveryPage {
            domains: page
                .domains
                .into_iter()
                .map(|d| AukiDiscoveredDomain {
                    domain: d.domain.into(),
                    permissions: d
                        .permissions
                        .into_iter()
                        .map(|p| p.as_str().into())
                        .collect(),
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    }
    #[uniffi::method(default(cursor = None, organization = None, cancellation = None))]
    pub async fn for_portal_page(
        &self,
        portal: String,
        limit: u32,
        cursor: Option<String>,
        organization: Option<String>,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiPortalDomainPage, AukiSdkError> {
        let page = self
            .inner
            .for_portal_page(
                &parse_portal(&portal)?,
                organization.as_deref().unwrap_or("own"),
                limit as usize,
                cursor.as_deref(),
                &operation_token(cancellation),
            )
            .await
            .map_err(data_error)?;
        Ok(AukiPortalDomainPage {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor,
            paginated: page.paginated,
        })
    }
    #[uniffi::method(default(cursor = None, cancellation = None))]
    pub async fn portals_page(
        &self,
        domain_id: String,
        limit: u32,
        cursor: Option<String>,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiPortalPage, AukiSdkError> {
        let page = self
            .inner
            .portals_page(
                parse_uuid(&domain_id, "Domain ID")?,
                limit as usize,
                cursor.as_deref(),
                &operation_token(cancellation),
            )
            .await
            .map_err(data_error)?;
        Ok(AukiPortalPage {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor,
            paginated: page.paginated,
        })
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn list(
        &self,
        query: AukiDomainListQuery,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiDomainPage, AukiSdkError> {
        let cancellation = operation_token(cancellation);
        self.inner
            .list_with_cancellation(&query.parse()?, &cancellation)
            .await
            .map(Into::into)
            .map_err(data_error)
    }

    #[uniffi::method(default(organization = None, cancellation = None))]
    pub async fn for_portal(
        &self,
        portal: String,
        organization: Option<String>,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Vec<AukiPortalDomain>, AukiSdkError> {
        let cancellation = operation_token(cancellation);
        self.inner
            .for_portal(
                &parse_portal(&portal)?,
                organization.as_deref().unwrap_or("own"),
                &cancellation,
            )
            .await
            .map(|values| values.into_iter().map(Into::into).collect())
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn portals(
        &self,
        domain_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Vec<AukiPortal>, AukiSdkError> {
        let cancellation = operation_token(cancellation);
        self.inner
            .portals(parse_uuid(&domain_id, "Domain ID")?, &cancellation)
            .await
            .map(|values| values.into_iter().map(Into::into).collect())
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn portal(
        &self,
        domain_id: String,
        portal: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiPortal, AukiSdkError> {
        let cancellation = operation_token(cancellation);
        self.inner
            .portal(
                parse_uuid(&domain_id, "Domain ID")?,
                &parse_portal(&portal)?,
                &cancellation,
            )
            .await
            .map(Into::into)
            .map_err(data_error)
    }
}

struct DataClientOwner {
    inner: DomainDataClient,
    closed: CancellationToken,
    completion: Mutex<Option<watch::Sender<Option<crate::CleanupResult>>>>,
    runtime: Mutex<Option<Handle>>,
}

impl DataClientOwner {
    fn remember_runtime(&self) -> Option<Handle> {
        let mut retained = self.runtime.lock();
        if retained.is_none() {
            *retained = Handle::try_current().ok();
        }
        retained.clone()
    }

    fn begin_close(&self) -> Option<watch::Receiver<Option<crate::CleanupResult>>> {
        self.closed.cancel();
        let mut completion = self.completion.lock();
        if let Some(sender) = completion.as_ref() {
            return Some(sender.subscribe());
        }
        let runtime = self.remember_runtime()?;
        let (sender, receiver) = watch::channel(None);
        *completion = Some(sender.clone());
        let inner = self.inner.clone();
        runtime.spawn(async move {
            inner.close().await;
            sender.send_replace(Some(Ok(())));
        });
        Some(receiver)
    }

    fn ensure_open(&self) -> Result<(), AukiSdkError> {
        if self.closed.is_cancelled() {
            Err(data_error(DataError::Closed))
        } else {
            Ok(())
        }
    }
}

impl Drop for DataClientOwner {
    fn drop(&mut self) {
        let _ = self.begin_close();
    }
}

#[derive(uniffi::Object)]
pub struct AukiDomainData {
    owner: DataClientOwner,
}

impl AukiDomainData {
    fn new(inner: DomainDataClient) -> Self {
        Self {
            owner: DataClientOwner {
                inner,
                closed: CancellationToken::new(),
                completion: Mutex::new(None),
                runtime: Mutex::new(Handle::try_current().ok()),
            },
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDomainData {
    pub fn domain_id(&self) -> String {
        self.owner.inner.domain_id().to_string()
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        let completion = self
            .owner
            .begin_close()
            .expect("UniFFI async data close runs on the retained Tokio runtime");
        wait_cleanup(completion)
            .await
            .map_err(|error| invalid_input(format!("close Domain data client: {error}")))
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn list(
        &self,
        query: AukiDataListQuery,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Vec<AukiDataMetadata>, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .list_with_cancellation(&query.parse()?, &cancellation)
            .await
            .map(|values| values.into_iter().map(Into::into).collect())
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn get(
        &self,
        data_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiDataMetadata, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .get_with_cancellation(parse_uuid(&data_id, "data ID")?, &cancellation)
            .await
            .map(Into::into)
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn read(
        &self,
        data_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Vec<u8>, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .read_with_cancellation(parse_uuid(&data_id, "data ID")?, &cancellation)
            .await
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn write(
        &self,
        target: AukiDataWriteTarget,
        bytes: Vec<u8>,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiDataMetadata, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .write_with_cancellation(target.as_write()?, &bytes, &cancellation)
            .await
            .map(Into::into)
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn delete(
        &self,
        data_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<(), AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .delete_with_cancellation(parse_uuid(&data_id, "data ID")?, &cancellation)
            .await
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn poses(
        &self,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Vec<AukiPortalPose>, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .poses(&cancellation)
            .await
            .map(|values| values.into_iter().map(Into::into).collect())
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn pose(
        &self,
        portal: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<AukiPortalPose, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let cancellation = operation_token(cancellation);
        self.owner
            .inner
            .pose(&parse_portal(&portal)?, &cancellation)
            .await
            .map(Into::into)
            .map_err(data_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn start_download(
        &self,
        data_id: String,
        options: AukiTransferOptions,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Arc<AukiDataDownload>, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let data_id = parse_uuid(&data_id, "data ID")?;
        let options = options.parse()?;
        let token = operation_token(cancellation);
        let (chunk_sender, chunk_receiver) = mpsc::channel::<Vec<u8>>(1);
        let (completion_sender, completion) = watch::channel(None);
        let inner = self.owner.inner.clone();
        let task_token = token.clone();
        tokio::spawn(async move {
            let result = inner
                .read_to(data_id, options, &task_token, move |bytes| {
                    let chunk_sender = chunk_sender.clone();
                    async move {
                        chunk_sender
                            .send(bytes)
                            .await
                            .map_err(|_| DataError::Callback)
                    }
                })
                .await
                .map_err(TransferFailure::from);
            completion_sender.send_replace(Some(result));
        });
        Ok(Arc::new(AukiDataDownload {
            cancellation: token,
            chunks: AsyncMutex::new(chunk_receiver),
            completion,
        }))
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn start_upload(
        &self,
        target: AukiDataWriteTarget,
        size: u64,
        options: AukiTransferOptions,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<Arc<AukiDataUpload>, AukiSdkError> {
        self.owner.ensure_open()?;
        self.owner.remember_runtime();
        let options = options.parse()?;
        target.as_write()?;
        let token = operation_token(cancellation);
        let (request_sender, request_receiver) = mpsc::channel::<u64>(1);
        let (chunk_sender, chunk_receiver) = mpsc::channel::<Vec<u8>>(1);
        let chunk_receiver = Arc::new(AsyncMutex::new(chunk_receiver));
        let (completion_sender, completion) = watch::channel(None);
        let inner = self.owner.inner.clone();
        let task_token = token.clone();
        tokio::spawn(async move {
            let result = inner
                .write_stream(
                    target.as_write().expect("validated target"),
                    size,
                    options,
                    &task_token,
                    move |maximum| {
                        let request_sender = request_sender.clone();
                        let chunk_receiver = Arc::clone(&chunk_receiver);
                        async move {
                            request_sender
                                .send(maximum as u64)
                                .await
                                .map_err(|_| DataError::Callback)?;
                            chunk_receiver
                                .lock()
                                .await
                                .recv()
                                .await
                                .ok_or(DataError::Callback)
                        }
                    },
                )
                .await
                .map(AukiDataMetadata::from)
                .map_err(TransferFailure::from);
            completion_sender.send_replace(Some(result));
        });
        Ok(Arc::new(AukiDataUpload {
            cancellation: token,
            next_gate: AsyncMutex::new(()),
            requests: AsyncMutex::new(request_receiver),
            chunks: chunk_sender,
            requested: Mutex::new(None),
            completion,
        }))
    }
}

async fn wait_completion<T: Clone>(
    completion: &watch::Receiver<Option<Result<T, TransferFailure>>>,
) -> Result<T, AukiSdkError> {
    let mut completion = completion.clone();
    loop {
        if let Some(result) = completion.borrow_and_update().clone() {
            return result.map_err(Into::into);
        }
        if completion.changed().await.is_err() {
            return Err(invalid_input("Domain data transfer ended without a result"));
        }
    }
}

#[derive(uniffi::Object)]
pub struct AukiDataDownload {
    cancellation: CancellationToken,
    chunks: AsyncMutex<mpsc::Receiver<Vec<u8>>>,
    completion: watch::Receiver<Option<Result<u64, TransferFailure>>>,
}

impl Drop for AukiDataDownload {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDataDownload {
    pub async fn next(&self) -> Result<Option<Vec<u8>>, AukiSdkError> {
        match self.chunks.lock().await.recv().await {
            Some(chunk) => Ok(Some(chunk)),
            None => wait_completion(&self.completion).await.map(|_| None),
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        self.cancellation.cancel();
        match wait_completion(&self.completion).await {
            Ok(_) => Ok(()),
            Err(AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Cancelled,
                ..
            }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[derive(uniffi::Object)]
pub struct AukiDataUpload {
    cancellation: CancellationToken,
    next_gate: AsyncMutex<()>,
    requests: AsyncMutex<mpsc::Receiver<u64>>,
    chunks: mpsc::Sender<Vec<u8>>,
    requested: Mutex<Option<u64>>,
    completion: watch::Receiver<Option<Result<AukiDataMetadata, TransferFailure>>>,
}

impl Drop for AukiDataUpload {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDataUpload {
    pub async fn next_maximum(&self) -> Result<Option<u64>, AukiSdkError> {
        let _next = self.next_gate.lock().await;
        if self.requested.lock().is_some() {
            return Err(invalid_input(
                "push the requested upload chunk before asking again",
            ));
        }
        match self.requests.lock().await.recv().await {
            Some(maximum) => {
                *self.requested.lock() = Some(maximum);
                Ok(Some(maximum))
            }
            None => wait_completion(&self.completion).await.map(|_| None),
        }
    }

    pub async fn push(&self, bytes: Vec<u8>) -> Result<(), AukiSdkError> {
        let maximum = self
            .requested
            .lock()
            .take()
            .ok_or_else(|| invalid_input("ask for the next upload maximum before pushing"))?;
        if bytes.len() as u64 > maximum {
            self.cancellation.cancel();
            return Err(invalid_input(
                "upload chunk must be no larger than the requested maximum",
            ));
        }
        self.chunks
            .send(bytes)
            .await
            .map_err(|_| invalid_input("upload is no longer accepting chunks"))
    }

    pub async fn result(&self) -> Result<AukiDataMetadata, AukiSdkError> {
        if self.requested.lock().is_some() {
            return Err(invalid_input(
                "push the requested upload chunk before awaiting result",
            ));
        }
        wait_completion(&self.completion).await
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        self.cancellation.cancel();
        match wait_completion(&self.completion).await {
            Ok(_) => Ok(()),
            Err(AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Cancelled,
                ..
            }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[uniffi::export]
impl AukiSession {
    pub fn domains(&self) -> Arc<AukiDomains> {
        Arc::new(AukiDomains {
            inner: RustDomains::new(self.session.clone()),
        })
    }

    pub fn data(&self, domain_id: String) -> Result<Arc<AukiDomainData>, AukiSdkError> {
        let inner = DataFactory::new(self.session.clone())
            .map_err(data_error)?
            .in_domain(parse_uuid(&domain_id, "Domain ID")?);
        Ok(Arc::new(AukiDomainData::new(inner)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use auki_sdk_rs::AuthError;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use httpmock::{
        Method::{DELETE, GET, POST, PUT},
        MockServer,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn data_errors_preserve_kind_and_http_status() {
        let error = data_error(DataError::HttpStatus { status: 403 });
        assert!(matches!(
            error,
            AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Http,
                status: Some(403),
                ..
            }
        ));

        let error = data_error(DataError::Auth(AuthError::Persistence));
        assert!(matches!(
            error,
            AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Authentication,
                auth_kind: Some(AukiAuthFailureKind::Persistence),
                ..
            }
        ));
    }

    #[test]
    fn query_and_transfer_options_apply_documented_defaults() {
        let query = AukiDomainListQuery::default().parse().unwrap();
        assert_eq!(
            (query.organization.as_str(), query.limit, query.offset),
            ("own", 50, 0)
        );
        let options = AukiTransferOptions::default().parse().unwrap();
        assert_eq!(options.max_bytes, DEFAULT_TRANSFER_MAX_BYTES);
        assert_eq!(
            options.max_chunk_bytes as u64,
            DEFAULT_TRANSFER_MAX_CHUNK_BYTES
        );
    }

    #[test]
    fn cancellation_children_follow_the_host_token() {
        let host = AukiCancellation::new();
        let operation = operation_token(Some(host.clone()));
        host.cancel();
        assert!(operation.is_cancelled());
    }

    #[tokio::test]
    async fn download_keeps_only_one_unconsumed_chunk() {
        let cancellation = CancellationToken::new();
        let (sender, receiver) = mpsc::channel(1);
        sender.send(vec![1]).await.unwrap();
        let sent_second = Arc::new(AtomicBool::new(false));
        let producer = tokio::spawn({
            let sent_second = Arc::clone(&sent_second);
            async move {
                sender.send(vec![2]).await.unwrap();
                sent_second.store(true, Ordering::Release);
            }
        });
        tokio::task::yield_now().await;
        assert!(!sent_second.load(Ordering::Acquire));
        let (completion_sender, completion) = watch::channel(None);
        let download = AukiDataDownload {
            cancellation,
            chunks: AsyncMutex::new(receiver),
            completion,
        };
        assert_eq!(download.next().await.unwrap(), Some(vec![1]));
        producer.await.unwrap();
        assert!(sent_second.load(Ordering::Acquire));
        completion_sender.send_replace(Some(Ok(2)));
        assert_eq!(download.next().await.unwrap(), Some(vec![2]));
        assert_eq!(download.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn cancelled_upload_close_waits_for_transfer_cleanup() {
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let cleaned = Arc::new(AtomicBool::new(false));
        let (completion_sender, completion) = watch::channel(None);
        tokio::spawn({
            let cleaned = Arc::clone(&cleaned);
            async move {
                task_cancellation.cancelled().await;
                tokio::task::yield_now().await;
                cleaned.store(true, Ordering::Release);
                completion_sender
                    .send_replace(Some(Err(TransferFailure::from(DataError::Cancelled))));
            }
        });
        let (request_sender, request_receiver) = mpsc::channel(1);
        let (chunk_sender, _chunk_receiver) = mpsc::channel(1);
        let upload = AukiDataUpload {
            cancellation,
            next_gate: AsyncMutex::new(()),
            requests: AsyncMutex::new(request_receiver),
            chunks: chunk_sender,
            requested: Mutex::new(None),
            completion,
        };
        drop(request_sender);
        upload.close().await.unwrap();
        assert!(cleaned.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn upload_close_surfaces_multipart_cleanup_failure() {
        let cancellation = CancellationToken::new();
        let (_completion_sender, completion) =
            watch::channel(Some(Err(TransferFailure::from(DataError::Cleanup {
                operation: Box::new(DataError::Cancelled),
                cleanup: Box::new(DataError::Transport),
            }))));
        let (_request_sender, request_receiver) = mpsc::channel(1);
        let (chunk_sender, _chunk_receiver) = mpsc::channel(1);
        let upload = AukiDataUpload {
            cancellation,
            next_gate: AsyncMutex::new(()),
            requests: AsyncMutex::new(request_receiver),
            chunks: chunk_sender,
            requested: Mutex::new(None),
            completion,
        };
        assert!(matches!(
            upload.close().await,
            Err(AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Cleanup,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn local_session_exercises_pagination_crud_status_renewal_and_upload_abort() {
        let server = MockServer::start_async().await;
        let domain = Uuid::new_v4();
        let data_id = Uuid::new_v4();
        let denied_id = Uuid::new_v4();
        let renewal_id = Uuid::new_v4();
        let upload_id = Uuid::new_v4();
        let metadata = json!({
            "id": data_id,
            "domain_id": domain,
            "name": "report",
            "data_type": "my-app.report.v1",
            "size": 7,
            "created_at": "2026-09-01T00:00:00Z",
            "updated_at": "2026-09-01T00:00:00Z"
        });
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/login");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"user-api", "refresh_token":"user-refresh"}));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/service/domains-access-token");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"dds-service"}));
            })
            .await;
        let page_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/api/v1/domains")
                    .query_param("org", "own")
                    .query_param("issue_token", "false")
                    .query_param("limit", "2")
                    .query_param("offset", "4")
                    .header("posemesh-client-id", "swift-installation");
                then.header("content-type", "application/json")
                    .json_body(json!({
                        "domains":[{"id":domain,"name":"Test","organization_id":null}],
                        "total":5,"limit":2,"offset":4
                    }));
            })
            .await;
        let grant = format!(
            "e30.{}.fixture-signature",
            URL_SAFE_NO_PAD.encode(
                json!({
                    "iss":"dds",
                    "domain_id":domain,
                    "aud":["dds",server.base_url()],
                    "exp":chrono::Utc::now().timestamp()+3600
                })
                .to_string()
            )
        );
        let auth_mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(format!("/api/v1/domains/{domain}/auth"))
                    .header("authorization", "Bearer dds-service")
                    .header("posemesh-client-id", "swift-installation");
                then.header("content-type", "application/json")
                    .json_body(json!({
                        "id":domain,
                        "domain_server":{"url":server.base_url()},
                        "access_token":grant
                    }));
            })
            .await;
        let data_path = format!("/api/v1/domains/{domain}/data");
        let list_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(&data_path)
                    .query_param("data_type", "my-app.report.v1");
                then.header("content-type", "application/json")
                    .json_body(json!({"data":[metadata.clone()]}));
            })
            .await;
        let read_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!("{data_path}/{data_id}"))
                    .query_param("raw", "true");
                then.body("payload");
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/api/v1/info");
                then.header("content-type", "application/json")
                    .json_body(json!({
                        "upload":{
                            "request_max_bytes":4096,
                            "domain_data_max_bytes":10000,
                            "multipart":{"enabled":true,"part_size_bytes":4}
                        }
                    }));
            })
            .await;
        let write_mock = server
            .mock_async(|when, then| {
                when.method(POST).path(&data_path);
                then.header("content-type", "application/json")
                    .json_body(json!({"data":[metadata.clone()]}));
            })
            .await;
        let delete_mock = server
            .mock_async(|when, then| {
                when.method(DELETE).path(format!("{data_path}/{data_id}"));
                then.status(200);
            })
            .await;
        let denied_mock = server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{data_path}/{denied_id}"));
                then.status(403);
            })
            .await;
        let renewal_mock = server
            .mock_async(|when, then| {
                when.method(GET).path(format!("{data_path}/{renewal_id}"));
                then.status(401);
            })
            .await;
        let initiate_mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(format!("{data_path}/multipart"))
                    .query_param("uploads", "");
                then.header("content-type", "application/json")
                    .json_body(json!({
                        "upload_id":upload_id,
                        "data_id":data_id,
                        "part_size":4,
                        "expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()
                    }));
            })
            .await;
        let part_mock = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path(format!("{data_path}/multipart"))
                    .query_param("uploadId", upload_id.to_string())
                    .query_param("partNumber", "1")
                    .body("abcd");
                then.header("content-type", "application/json")
                    .json_body(json!({"etag":"part-one"}));
            })
            .await;
        let abort_mock = server
            .mock_async(|when, then| {
                when.method(DELETE)
                    .path(format!("{data_path}/multipart"))
                    .query_param("uploadId", upload_id.to_string());
                then.status(200);
            })
            .await;

        let session = AukiSession::login_data_with_environment(
            server.base_url(),
            server.base_url(),
            "user@example.com".into(),
            "password".into(),
            Some("swift-installation".into()),
        )
        .await
        .unwrap();
        let page = session
            .domains()
            .list(
                AukiDomainListQuery {
                    limit: Some(2),
                    offset: Some(4),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(page.domains[0].id, domain.to_string());
        assert_eq!(page.total, 5);
        let data = session.data(domain.to_string()).unwrap();
        assert_eq!(
            data.list(
                AukiDataListQuery {
                    data_type: Some("my-app.report.v1".into()),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap()[0]
                .id,
            data_id.to_string()
        );
        assert_eq!(
            data.read(data_id.to_string(), None).await.unwrap(),
            b"payload"
        );
        assert_eq!(
            data.write(
                AukiDataWriteTarget::Named {
                    name: "report".into(),
                    data_type: "my-app.report.v1".into(),
                },
                b"payload".to_vec(),
                None,
            )
            .await
            .unwrap()
            .id,
            data_id.to_string()
        );
        data.delete(data_id.to_string(), None).await.unwrap();
        assert!(matches!(
            data.get(denied_id.to_string(), None).await,
            Err(AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Http,
                status: Some(403),
                ..
            })
        ));
        assert!(matches!(
            data.get(renewal_id.to_string(), None).await,
            Err(AukiSdkError::DomainData {
                kind: AukiDataFailureKind::Http,
                status: Some(401),
                ..
            })
        ));
        let upload = data
            .start_upload(
                AukiDataWriteTarget::ById {
                    id: data_id.to_string(),
                },
                7,
                AukiTransferOptions {
                    max_bytes: Some(10000),
                    max_chunk_bytes: Some(16),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(upload.next_maximum().await.unwrap(), Some(4));
        upload.push(b"abcd".to_vec()).await.unwrap();
        assert_eq!(upload.next_maximum().await.unwrap(), Some(3));
        upload.cancel();
        upload.close().await.unwrap();
        data.close().await.unwrap();
        session.close().await;

        page_mock.assert_calls_async(1).await;
        list_mock.assert_calls_async(1).await;
        read_mock.assert_calls_async(1).await;
        write_mock.assert_calls_async(1).await;
        delete_mock.assert_calls_async(1).await;
        denied_mock.assert_calls_async(1).await;
        renewal_mock.assert_calls_async(2).await;
        initiate_mock.assert_calls_async(1).await;
        part_mock.assert_calls_async(1).await;
        abort_mock.assert_calls_async(1).await;
        assert_eq!(auth_mock.calls_async().await, 2);
    }
}
