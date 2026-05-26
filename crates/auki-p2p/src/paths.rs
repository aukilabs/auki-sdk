//! Pure Get and Subscribe path orchestration.

use crate::{
    AukiP2pConfig, OfferLoadReport, PeerRelationship, RelationshipFailureRecord,
    RelationshipFailureScope, RelationshipPathStatus,
};
use auki_protocol::v1::{
    error,
    frame::{FrameError, decode_json_frame, decode_length},
    get::{GetRequest, GetRequestError, GetResponse, GetResponseBody, GetResponseError},
    message::SpatialMessage,
    subscribe::{
        SubscribeAccept, SubscribeAcceptError, SubscribeDataError, SubscribeEnd, SubscribeEndError,
        SubscribeEndReason, SubscribeRequest, SubscribeRequestError, SubscribeStartResult,
        SubscribeStartResultBody, SubscribeStartResultError,
    },
};
use libp2p_identity::PeerId;
use serde_json::Value;
use std::fmt;

const GET_PATH_TYPE: &str = "get";
const SUBSCRIBE_PATH_TYPE: &str = "subscribe";
const GET_REQUESTED: &str = "requested";
const GET_SUCCEEDED: &str = "succeeded";
const PATH_FAILED: &str = "failed";
const SUBSCRIBE_STARTING: &str = "starting";
const SUBSCRIBE_ACTIVE: &str = "active";
const SUBSCRIBE_ENDING: &str = "ending";
const SUBSCRIBE_ENDED: &str = "ended";

/// Internal client boundary for one Get operation.
pub trait GetClient {
    /// Execute one high-level Get request and return one encoded Get response frame.
    fn get(&mut self, peer_id: PeerId, request: GetRequest) -> Result<Vec<u8>, PathClientError>;
}

/// Internal client boundary for one Subscribe start operation.
pub trait SubscribeClient {
    /// Start one subscription and return one encoded Subscribe accept/reject frame.
    fn subscribe(
        &mut self,
        peer_id: PeerId,
        request: SubscribeRequest,
    ) -> Result<Vec<u8>, PathClientError>;
}

/// Transport/client failure while running a path operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathClientError {
    /// Stable failure code.
    pub code: &'static str,
    /// Local diagnostic message.
    pub message: String,
    /// Retry hint for local status only.
    pub retryable: bool,
}

/// Runtime context for one path operation.
#[derive(Debug, Clone, Copy)]
pub struct PathContext<'a> {
    /// Runtime config and limits.
    pub config: &'a AukiP2pConfig,
    /// Current UTC time for status path/failure records.
    pub now: &'a str,
}

/// High-level Get input. Callers do not pass protocol frames.
#[derive(Debug, Clone, PartialEq)]
pub struct GetInput {
    /// Optional implementation-defined path id.
    pub path_id: Option<String>,
    /// Requested producing domain.
    pub domain_id: String,
    /// Requested producer-scoped offer id.
    pub offer_id: String,
    /// Optional offer-kind-specific params.
    pub params: Option<Value>,
    /// Accepted payload types. Empty means use the loaded offer payload type.
    pub accepted_payload_types: Vec<String>,
    /// Optional requester raw-payload byte limit.
    pub max_payload_bytes: Option<u64>,
}

/// High-level Subscribe input. Callers do not pass protocol frames.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeInput {
    /// Optional implementation-defined path id.
    pub path_id: Option<String>,
    /// Requested producing domain.
    pub domain_id: String,
    /// Requested producer-scoped offer id.
    pub offer_id: String,
    /// Optional offer-kind-specific params.
    pub params: Option<Value>,
    /// Accepted payload types. Empty means use the loaded offer payload type.
    pub accepted_payload_types: Vec<String>,
    /// Optional requester serialized message byte limit.
    pub max_message_bytes: Option<u64>,
}

/// Successful Get outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct GetOutcome {
    /// Path id used for status tracking.
    pub path_id: String,
    /// Validated response message.
    pub message: SpatialMessage,
}

/// Accepted subscription handle.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionHandle {
    path_id: String,
    peer_id: PeerId,
    request: SubscribeRequest,
    accept: SubscribeAccept,
    max_message_bytes: Option<u64>,
    started_at: String,
    last_message_at: Option<String>,
    next_expected_sequence: Option<u64>,
    last_sequence: Option<u64>,
    sequence_gap_count: u64,
    last_envelope_failure: Option<RelationshipFailureRecord>,
    last_payload_failure: Option<RelationshipFailureRecord>,
    last_failure: Option<RelationshipFailureRecord>,
}

/// Path orchestration failure.
#[derive(Debug, Clone, PartialEq)]
pub enum PathOrchestrationError {
    /// Requested offer was not loaded.
    UnknownOffer {
        /// Domain id.
        domain_id: String,
        /// Offer id.
        offer_id: String,
    },
    /// Requested offer is loaded but locally unusable.
    OfferUnusable {
        /// Domain id.
        domain_id: String,
        /// Offer id.
        offer_id: String,
        /// Stable failure code.
        code: String,
    },
    /// Get request construction failed.
    GetRequest(GetRequestError),
    /// Get client failed.
    GetClient(PathClientError),
    /// Get response frame failed.
    GetFrame(FrameError),
    /// Get response frame contained trailing bytes.
    GetTrailingFrameBytes {
        /// Bytes consumed by the decoded frame.
        consumed: usize,
        /// Total bytes supplied.
        total: usize,
    },
    /// Get response validation failed.
    GetResponse(GetResponseError),
    /// Remote Get response carried a structured failure.
    GetRejected {
        /// Stable remote failure code.
        code: String,
    },
    /// Subscribe request construction failed.
    SubscribeRequest(SubscribeRequestError),
    /// Subscribe client failed.
    SubscribeClient(PathClientError),
    /// Subscribe start frame failed.
    SubscribeStartFrame(FrameError),
    /// Subscribe start frame contained trailing bytes.
    SubscribeStartTrailingFrameBytes {
        /// Bytes consumed by the decoded frame.
        consumed: usize,
        /// Total bytes supplied.
        total: usize,
    },
    /// Subscribe start result failed to parse.
    SubscribeStart(SubscribeStartResultError),
    /// Subscribe accept failed request negotiation validation.
    SubscribeAccept(SubscribeAcceptError),
    /// Remote Subscribe start returned a structured rejection.
    SubscribeRejected {
        /// Stable remote failure code.
        code: String,
    },
    /// Subscribe data frame failed.
    SubscribeDataFrame(FrameError),
    /// Subscribe data frame contained trailing bytes.
    SubscribeDataTrailingFrameBytes {
        /// Bytes consumed by the decoded frame.
        consumed: usize,
        /// Total bytes supplied.
        total: usize,
    },
    /// Subscribe data message failed validation.
    SubscribeData(SubscribeDataError),
    /// Subscribe end frame failed.
    SubscribeEndFrame(FrameError),
    /// Subscribe end frame contained trailing bytes.
    SubscribeEndTrailingFrameBytes {
        /// Bytes consumed by the decoded frame.
        consumed: usize,
        /// Total bytes supplied.
        total: usize,
    },
    /// Subscribe end message failed validation.
    SubscribeEnd(SubscribeEndError),
}

impl PathClientError {
    /// Create a client failure.
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

impl<'a> PathContext<'a> {
    /// Create a path context.
    pub fn new(config: &'a AukiP2pConfig, now: &'a str) -> Self {
        Self { config, now }
    }
}

impl GetInput {
    /// Create high-level Get input.
    pub fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            path_id: None,
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
            params: None,
            accepted_payload_types: Vec::new(),
            max_payload_bytes: None,
        }
    }
}

impl SubscribeInput {
    /// Create high-level Subscribe input.
    pub fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            path_id: None,
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
            params: None,
            accepted_payload_types: Vec::new(),
            max_message_bytes: None,
        }
    }
}

impl SubscriptionHandle {
    /// Path id used for status tracking.
    pub fn path_id(&self) -> &str {
        &self.path_id
    }

    /// Remote peer id.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Accepted payload type for this subscription.
    pub fn payload_type(&self) -> &str {
        &self.accept.payload.payload_type
    }

    /// Last accepted sequence value.
    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    /// Observed sequence-gap count.
    pub fn sequence_gap_count(&self) -> u64 {
        self.sequence_gap_count
    }
}

impl PathOrchestrationError {
    /// Stable failure code for status reporting.
    pub fn failure_code(&self) -> String {
        match self {
            Self::UnknownOffer { .. } => error::OFFER_UNKNOWN_OFFER.to_owned(),
            Self::OfferUnusable { code, .. } => code.clone(),
            Self::GetRequest(error) => error.failure_code().to_owned(),
            Self::GetClient(error) => error.code.to_owned(),
            Self::GetFrame(FrameError::BodyTooLarge { .. }) => {
                error::MESSAGE_PAYLOAD_TOO_LARGE.to_owned()
            }
            Self::GetFrame(_) | Self::GetTrailingFrameBytes { .. } => {
                error::TRANSPORT_FAILED.to_owned()
            }
            Self::GetResponse(GetResponseError::ErrorResponse { code })
            | Self::GetRejected { code } => code.clone(),
            Self::GetResponse(error) => error.failure_code().to_owned(),
            Self::SubscribeRequest(error) => error.failure_code().to_owned(),
            Self::SubscribeClient(error) => error.code.to_owned(),
            Self::SubscribeStartFrame(FrameError::BodyTooLarge { .. })
            | Self::SubscribeDataFrame(FrameError::BodyTooLarge { .. })
            | Self::SubscribeData(SubscribeDataError::MessageTooLarge { .. }) => {
                error::MESSAGE_PAYLOAD_TOO_LARGE.to_owned()
            }
            Self::SubscribeStartFrame(_)
            | Self::SubscribeStartTrailingFrameBytes { .. }
            | Self::SubscribeDataFrame(_)
            | Self::SubscribeDataTrailingFrameBytes { .. }
            | Self::SubscribeEndFrame(_)
            | Self::SubscribeEndTrailingFrameBytes { .. } => error::TRANSPORT_FAILED.to_owned(),
            Self::SubscribeStart(error) => error.failure_code().to_owned(),
            Self::SubscribeAccept(error) => error.failure_code().to_owned(),
            Self::SubscribeRejected { code } => code.clone(),
            Self::SubscribeData(error) => error.failure_code().to_owned(),
            Self::SubscribeEnd(error) => error.failure_code().to_owned(),
        }
    }

    fn retryable(&self) -> Option<bool> {
        match self {
            Self::GetClient(error) | Self::SubscribeClient(error) => Some(error.retryable),
            Self::GetFrame(_)
            | Self::GetTrailingFrameBytes { .. }
            | Self::SubscribeStartFrame(_)
            | Self::SubscribeStartTrailingFrameBytes { .. }
            | Self::SubscribeDataFrame(_)
            | Self::SubscribeDataTrailingFrameBytes { .. }
            | Self::SubscribeEndFrame(_)
            | Self::SubscribeEndTrailingFrameBytes { .. } => Some(true),
            _ => Some(false),
        }
    }
}

impl fmt::Display for PathOrchestrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOffer {
                domain_id,
                offer_id,
            } => write!(f, "unknown offer {domain_id}/{offer_id}"),
            Self::OfferUnusable {
                domain_id,
                offer_id,
                code,
            } => write!(f, "offer {domain_id}/{offer_id} is unusable: {code}"),
            Self::GetRequest(error) => write!(f, "get request failed: {error}"),
            Self::GetClient(error) => write!(f, "get client failed: {}", error.message),
            Self::GetFrame(error) => write!(f, "get response frame failed: {error}"),
            Self::GetTrailingFrameBytes { consumed, total } => write!(
                f,
                "get response frame had trailing bytes: consumed {consumed}, total {total}"
            ),
            Self::GetResponse(error) => write!(f, "get response failed validation: {error}"),
            Self::GetRejected { code } => write!(f, "get rejected with {code}"),
            Self::SubscribeRequest(error) => write!(f, "subscribe request failed: {error}"),
            Self::SubscribeClient(error) => write!(f, "subscribe client failed: {}", error.message),
            Self::SubscribeStartFrame(error) => {
                write!(f, "subscribe start frame failed: {error}")
            }
            Self::SubscribeStartTrailingFrameBytes { consumed, total } => write!(
                f,
                "subscribe start frame had trailing bytes: consumed {consumed}, total {total}"
            ),
            Self::SubscribeStart(error) => write!(f, "subscribe start failed: {error}"),
            Self::SubscribeAccept(error) => write!(f, "subscribe accept failed: {error}"),
            Self::SubscribeRejected { code } => write!(f, "subscribe rejected with {code}"),
            Self::SubscribeDataFrame(error) => write!(f, "subscribe data frame failed: {error}"),
            Self::SubscribeDataTrailingFrameBytes { consumed, total } => write!(
                f,
                "subscribe data frame had trailing bytes: consumed {consumed}, total {total}"
            ),
            Self::SubscribeData(error) => write!(f, "subscribe data failed: {error}"),
            Self::SubscribeEndFrame(error) => write!(f, "subscribe end frame failed: {error}"),
            Self::SubscribeEndTrailingFrameBytes { consumed, total } => write!(
                f,
                "subscribe end frame had trailing bytes: consumed {consumed}, total {total}"
            ),
            Self::SubscribeEnd(error) => write!(f, "subscribe end failed: {error}"),
        }
    }
}

impl std::error::Error for PathOrchestrationError {}

/// Run one high-level Get operation.
pub fn get<C: GetClient>(
    relationship: &mut PeerRelationship,
    offers: &OfferLoadReport,
    client: &mut C,
    input: GetInput,
    context: PathContext<'_>,
) -> Result<GetOutcome, PathOrchestrationError> {
    let path_id = input
        .path_id
        .clone()
        .unwrap_or_else(|| default_path_id(GET_PATH_TYPE, relationship.peer_id, &input));
    let mut path = base_path(
        &path_id,
        GET_PATH_TYPE,
        &input.domain_id,
        &input.offer_id,
        GET_REQUESTED,
        context.now,
    );
    relationship.upsert_path(path.clone(), context.config.limits.completed_path_history);

    let loaded = match resolve_loaded_offer(offers, &input.domain_id, &input.offer_id) {
        Ok(loaded) => loaded,
        Err(error) => {
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };
    let accepted_payload_types = effective_accepted_payload_types(
        &input.accepted_payload_types,
        &loaded.offer.payload.payload_type,
    );
    let max_payload_bytes = effective_limit(
        input.max_payload_bytes,
        context.config.limits.get_response_frame_body_bytes,
    );
    let request = match GetRequest::create(
        input.domain_id.clone(),
        input.offer_id.clone(),
        input.params.clone(),
        accepted_payload_types,
        max_payload_bytes,
    ) {
        Ok(request) => request,
        Err(error) => {
            let error = PathOrchestrationError::GetRequest(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };

    let frame = match client.get(relationship.peer_id, request.clone()) {
        Ok(frame) => frame,
        Err(error) => {
            let error = PathOrchestrationError::GetClient(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };
    let (value, _) = match decode_single_frame(
        &frame,
        context.config.limits.get_response_frame_body_bytes,
        PathFrameKind::Get,
    ) {
        Ok(decoded) => decoded,
        Err(error) => {
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };
    let response = match GetResponse::from_value(value) {
        Ok(response) => response,
        Err(error) => {
            let error = PathOrchestrationError::GetResponse(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };

    if let GetResponseBody::Error(remote_error) = &response.body {
        let error = PathOrchestrationError::GetRejected {
            code: remote_error.code.clone(),
        };
        fail_path(
            relationship,
            &mut path,
            &error,
            RelationshipFailureScope::Get,
            context,
        );
        return Err(error);
    }

    let message = match response
        .validate_success_for_request(&request, &loaded.offer.payload.payload_type)
        .cloned()
    {
        Ok(message) => message,
        Err(error) => {
            let error = PathOrchestrationError::GetResponse(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Get,
                context,
            );
            return Err(error);
        }
    };

    path.state = Some(GET_SUCCEEDED.to_owned());
    path.last_message_at = Some(context.now.to_owned());
    path.payload_type = Some(message.payload.payload_type.clone());
    path.last_sequence = message.sequence;
    relationship.upsert_path(path, context.config.limits.completed_path_history);

    Ok(GetOutcome { path_id, message })
}

/// Start one high-level Subscribe operation.
pub fn subscribe<C: SubscribeClient>(
    relationship: &mut PeerRelationship,
    offers: &OfferLoadReport,
    client: &mut C,
    input: SubscribeInput,
    context: PathContext<'_>,
) -> Result<SubscriptionHandle, PathOrchestrationError> {
    let path_id = input
        .path_id
        .clone()
        .unwrap_or_else(|| default_path_id(SUBSCRIBE_PATH_TYPE, relationship.peer_id, &input));
    let mut path = base_path(
        &path_id,
        SUBSCRIBE_PATH_TYPE,
        &input.domain_id,
        &input.offer_id,
        SUBSCRIBE_STARTING,
        context.now,
    );
    relationship.upsert_path(path.clone(), context.config.limits.completed_path_history);

    let loaded = match resolve_loaded_offer(offers, &input.domain_id, &input.offer_id) {
        Ok(loaded) => loaded,
        Err(error) => {
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };
    let accepted_payload_types = effective_accepted_payload_types(
        &input.accepted_payload_types,
        &loaded.offer.payload.payload_type,
    );
    let max_message_bytes = effective_limit(
        input.max_message_bytes,
        context.config.limits.subscribe_message_frame_body_bytes,
    );
    let request = match SubscribeRequest::create(
        input.domain_id.clone(),
        input.offer_id.clone(),
        input.params.clone(),
        accepted_payload_types,
        max_message_bytes,
    ) {
        Ok(request) => request,
        Err(error) => {
            let error = PathOrchestrationError::SubscribeRequest(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };

    let frame = match client.subscribe(relationship.peer_id, request.clone()) {
        Ok(frame) => frame,
        Err(error) => {
            let error = PathOrchestrationError::SubscribeClient(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };
    let (value, _) = match decode_single_frame(
        &frame,
        context.config.limits.subscribe_message_frame_body_bytes,
        PathFrameKind::SubscribeStart,
    ) {
        Ok(decoded) => decoded,
        Err(error) => {
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };
    let start = match SubscribeStartResult::from_value(value) {
        Ok(start) => start,
        Err(error) => {
            let error = PathOrchestrationError::SubscribeStart(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };

    let accept = match start.body {
        SubscribeStartResultBody::Accept(accept) => accept,
        SubscribeStartResultBody::Reject(reject) => {
            let error = PathOrchestrationError::SubscribeRejected {
                code: reject.error.code.clone(),
            };
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };

    if let Err(error) = accept.validate_for_request(&request) {
        let error = PathOrchestrationError::SubscribeAccept(error);
        fail_path(
            relationship,
            &mut path,
            &error,
            RelationshipFailureScope::Subscribe,
            context,
        );
        return Err(error);
    }

    path.state = Some(SUBSCRIBE_ACTIVE.to_owned());
    path.payload_type = Some(accept.payload.payload_type.clone());
    relationship.upsert_path(path, context.config.limits.completed_path_history);

    let next_expected_sequence = accept.initial_sequence;
    Ok(SubscriptionHandle {
        path_id,
        peer_id: relationship.peer_id,
        request,
        accept,
        max_message_bytes,
        started_at: context.now.to_owned(),
        last_message_at: None,
        next_expected_sequence,
        last_sequence: None,
        sequence_gap_count: 0,
        last_envelope_failure: None,
        last_payload_failure: None,
        last_failure: None,
    })
}

/// Validate and accept one Subscribe data-message frame.
pub fn accept_subscribe_data_frame(
    relationship: &mut PeerRelationship,
    subscription: &mut SubscriptionHandle,
    frame: &[u8],
    context: PathContext<'_>,
) -> Result<SpatialMessage, PathOrchestrationError> {
    let mut path = subscription.path_status(SUBSCRIBE_ACTIVE);
    let (value, body_len) = match decode_single_frame(
        frame,
        context.config.limits.subscribe_message_frame_body_bytes,
        PathFrameKind::SubscribeData,
    ) {
        Ok(decoded) => decoded,
        Err(error) => {
            let failure = fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Message,
                context,
            );
            subscription.record_failure(failure, RelationshipFailureScope::Message);
            return Err(error);
        }
    };
    let message = match SpatialMessage::from_value(value) {
        Ok(message) => message,
        Err(error) => {
            let error =
                PathOrchestrationError::SubscribeData(SubscribeDataError::InvalidMessage(error));
            let failure = fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Message,
                context,
            );
            subscription.record_failure(failure, RelationshipFailureScope::Message);
            return Err(error);
        }
    };

    if let Err(error) = subscription.accept.validate_data_message_with_body_len(
        &message,
        body_len,
        subscription.max_message_bytes,
    ) {
        let error = PathOrchestrationError::SubscribeData(error);
        let failure = fail_path(
            relationship,
            &mut path,
            &error,
            RelationshipFailureScope::Message,
            context,
        );
        subscription.record_failure(failure, RelationshipFailureScope::Message);
        return Err(error);
    }

    subscription.accept_message_sequence(relationship, &message, context);
    Ok(message)
}

/// Validate and apply one Subscribe end frame.
pub fn end_subscription_from_frame(
    relationship: &mut PeerRelationship,
    subscription: &mut SubscriptionHandle,
    frame: &[u8],
    context: PathContext<'_>,
) -> Result<SubscribeEnd, PathOrchestrationError> {
    let mut path = subscription.path_status(SUBSCRIBE_ENDING);
    relationship.upsert_path(path.clone(), context.config.limits.completed_path_history);

    let (value, _) = match decode_single_frame(
        frame,
        context.config.limits.subscribe_message_frame_body_bytes,
        PathFrameKind::SubscribeEnd,
    ) {
        Ok(decoded) => decoded,
        Err(error) => {
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };
    let end = match SubscribeEnd::from_value(value) {
        Ok(end) => end,
        Err(error) => {
            let error = PathOrchestrationError::SubscribeEnd(error);
            fail_path(
                relationship,
                &mut path,
                &error,
                RelationshipFailureScope::Subscribe,
                context,
            );
            return Err(error);
        }
    };
    if let Err(error) = end.validate_for_offer(
        &subscription.request.domain_id,
        &subscription.request.offer_id,
    ) {
        let error = PathOrchestrationError::SubscribeEnd(error);
        fail_path(
            relationship,
            &mut path,
            &error,
            RelationshipFailureScope::Subscribe,
            context,
        );
        return Err(error);
    }

    if end.reason == SubscribeEndReason::Error || end.error.is_some() {
        let error = PathOrchestrationError::SubscribeRejected {
            code: end
                .error
                .as_ref()
                .map(|error| error.code.clone())
                .unwrap_or_else(|| error::TRANSPORT_FAILED.to_owned()),
        };
        fail_path(
            relationship,
            &mut path,
            &error,
            RelationshipFailureScope::Subscribe,
            context,
        );
    } else {
        path.state = Some(SUBSCRIBE_ENDED.to_owned());
        relationship.upsert_path(path, context.config.limits.completed_path_history);
    }

    Ok(end)
}

impl SubscriptionHandle {
    fn path_status(&self, state: &str) -> RelationshipPathStatus {
        RelationshipPathStatus {
            path_id: Some(self.path_id.clone()),
            path_type: Some(SUBSCRIBE_PATH_TYPE.to_owned()),
            domain_id: Some(self.request.domain_id.clone()),
            offer_id: Some(self.request.offer_id.clone()),
            state: Some(state.to_owned()),
            started_at: Some(self.started_at.clone()),
            last_message_at: self.last_message_at.clone(),
            payload_type: Some(self.accept.payload.payload_type.clone()),
            last_sequence: self.last_sequence,
            sequence_gap_count: self.sequence_gap_count,
            last_envelope_failure: self.last_envelope_failure.clone(),
            last_payload_failure: self.last_payload_failure.clone(),
            last_failure: self.last_failure.clone(),
            ..RelationshipPathStatus::default()
        }
    }

    fn accept_message_sequence(
        &mut self,
        relationship: &mut PeerRelationship,
        message: &SpatialMessage,
        context: PathContext<'_>,
    ) {
        if let Some(sequence) = message.sequence {
            if let Some(expected) = self.next_expected_sequence
                && sequence != expected
            {
                self.sequence_gap_count = self.sequence_gap_count.saturating_add(1);
                let path = self.path_status(SUBSCRIBE_ACTIVE);
                let failure = path_failure_record(
                    error::MESSAGE_SEQUENCE_GAP,
                    "subscribe sequence gap observed".to_owned(),
                    RelationshipFailureScope::Message,
                    relationship.peer_id,
                    &path,
                    context,
                    Some(false),
                );
                self.last_envelope_failure = Some(failure.clone());
                self.last_failure = Some(failure.clone());
                relationship
                    .record_failure(failure, context.config.limits.retained_status_failures);
            }
            self.last_sequence = Some(sequence);
            self.next_expected_sequence = sequence.checked_add(1);
        }

        self.last_message_at = Some(context.now.to_owned());
        let path = self.path_status(SUBSCRIBE_ACTIVE);
        relationship.upsert_path(path, context.config.limits.completed_path_history);
    }

    fn record_failure(
        &mut self,
        failure: RelationshipFailureRecord,
        scope: RelationshipFailureScope,
    ) {
        if matches!(scope, RelationshipFailureScope::Message) {
            if failure.code == error::MESSAGE_INVALID_PAYLOAD {
                self.last_payload_failure = Some(failure.clone());
            } else {
                self.last_envelope_failure = Some(failure.clone());
            }
        }
        self.last_failure = Some(failure);
    }
}

fn resolve_loaded_offer<'a>(
    offers: &'a OfferLoadReport,
    domain_id: &str,
    offer_id: &str,
) -> Result<&'a crate::LoadedRemoteOffer, PathOrchestrationError> {
    let loaded = offers.find_offer(domain_id, offer_id).map_err(|_| {
        PathOrchestrationError::UnknownOffer {
            domain_id: domain_id.to_owned(),
            offer_id: offer_id.to_owned(),
        }
    })?;
    if !loaded.usable {
        return Err(PathOrchestrationError::OfferUnusable {
            domain_id: domain_id.to_owned(),
            offer_id: offer_id.to_owned(),
            code: loaded
                .unusable_reason
                .unwrap_or(error::OFFER_LOAD_FAILED)
                .to_owned(),
        });
    }
    Ok(loaded)
}

fn effective_limit(requested: Option<u64>, local: u64) -> Option<u64> {
    Some(requested.map_or(local, |requested| requested.min(local)))
}

fn effective_accepted_payload_types(
    requested: &[String],
    loaded_payload_type: &str,
) -> Vec<String> {
    if requested.is_empty() {
        vec![loaded_payload_type.to_owned()]
    } else {
        requested.to_vec()
    }
}

trait PathInputLike {
    fn domain_id(&self) -> &str;
    fn offer_id(&self) -> &str;
}

impl PathInputLike for GetInput {
    fn domain_id(&self) -> &str {
        &self.domain_id
    }

    fn offer_id(&self) -> &str {
        &self.offer_id
    }
}

impl PathInputLike for SubscribeInput {
    fn domain_id(&self) -> &str {
        &self.domain_id
    }

    fn offer_id(&self) -> &str {
        &self.offer_id
    }
}

fn default_path_id(kind: &str, peer_id: PeerId, input: &impl PathInputLike) -> String {
    format!(
        "{kind}:{peer_id}:{}:{}",
        input.domain_id(),
        input.offer_id()
    )
}

fn base_path(
    path_id: &str,
    path_type: &str,
    domain_id: &str,
    offer_id: &str,
    state: &str,
    now: &str,
) -> RelationshipPathStatus {
    RelationshipPathStatus {
        path_id: Some(path_id.to_owned()),
        path_type: Some(path_type.to_owned()),
        domain_id: Some(domain_id.to_owned()),
        offer_id: Some(offer_id.to_owned()),
        state: Some(state.to_owned()),
        started_at: Some(now.to_owned()),
        ..RelationshipPathStatus::default()
    }
}

#[derive(Debug, Clone, Copy)]
enum PathFrameKind {
    Get,
    SubscribeStart,
    SubscribeData,
    SubscribeEnd,
}

fn decode_single_frame(
    frame: &[u8],
    max_body_len: u64,
    kind: PathFrameKind,
) -> Result<(Value, usize), PathOrchestrationError> {
    let (body_len, _) =
        decode_length(frame, max_body_len).map_err(|error| frame_error(kind, error))?;
    let (value, consumed) =
        decode_json_frame(frame, max_body_len).map_err(|error| frame_error(kind, error))?;
    if consumed != frame.len() {
        return Err(match kind {
            PathFrameKind::Get => PathOrchestrationError::GetTrailingFrameBytes {
                consumed,
                total: frame.len(),
            },
            PathFrameKind::SubscribeStart => {
                PathOrchestrationError::SubscribeStartTrailingFrameBytes {
                    consumed,
                    total: frame.len(),
                }
            }
            PathFrameKind::SubscribeData => {
                PathOrchestrationError::SubscribeDataTrailingFrameBytes {
                    consumed,
                    total: frame.len(),
                }
            }
            PathFrameKind::SubscribeEnd => PathOrchestrationError::SubscribeEndTrailingFrameBytes {
                consumed,
                total: frame.len(),
            },
        });
    }
    let body_len =
        usize::try_from(body_len).map_err(|_| frame_error(kind, FrameError::LengthOverflow))?;
    Ok((value, body_len))
}

fn frame_error(kind: PathFrameKind, error: FrameError) -> PathOrchestrationError {
    match kind {
        PathFrameKind::Get => PathOrchestrationError::GetFrame(error),
        PathFrameKind::SubscribeStart => PathOrchestrationError::SubscribeStartFrame(error),
        PathFrameKind::SubscribeData => PathOrchestrationError::SubscribeDataFrame(error),
        PathFrameKind::SubscribeEnd => PathOrchestrationError::SubscribeEndFrame(error),
    }
}

fn fail_path(
    relationship: &mut PeerRelationship,
    path: &mut RelationshipPathStatus,
    error: &PathOrchestrationError,
    scope: RelationshipFailureScope,
    context: PathContext<'_>,
) -> RelationshipFailureRecord {
    path.state = Some(PATH_FAILED.to_owned());
    let failure = path_failure_record(
        error.failure_code(),
        error.to_string(),
        scope,
        relationship.peer_id,
        path,
        context,
        error.retryable(),
    );
    if matches!(scope, RelationshipFailureScope::Message) {
        if failure.code == error::MESSAGE_INVALID_PAYLOAD {
            path.last_payload_failure = Some(failure.clone());
        } else {
            path.last_envelope_failure = Some(failure.clone());
        }
    }
    path.last_failure = Some(failure.clone());
    relationship.record_failure(
        failure.clone(),
        context.config.limits.retained_status_failures,
    );
    relationship.upsert_path(path.clone(), context.config.limits.completed_path_history);
    failure
}

fn path_failure_record(
    code: impl Into<String>,
    message: String,
    scope: RelationshipFailureScope,
    peer_id: PeerId,
    path: &RelationshipPathStatus,
    context: PathContext<'_>,
    retryable: Option<bool>,
) -> RelationshipFailureRecord {
    let mut failure = RelationshipFailureRecord::new(code, context.now, scope);
    failure.peer_id = Some(peer_id);
    failure.domain_id = path.domain_id.clone();
    failure.offer_id = path.offer_id.clone();
    failure.path_id = path.path_id.clone();
    failure.retryable = retryable;
    failure.message = Some(message);
    failure
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LoadedRemoteOffer, OfferCatalogLoadState, OfferLoadReport, PeerRelationshipState};
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        frame::encode_json_frame,
        get::GetResponse,
        message::{ErrorObject, SpatialMessage},
        offer::{Offer, OfferAccessMode, OfferStatus, PayloadDescriptor, RegistryReference},
        subscribe::{SubscribeAccept, SubscribeEndReason, SubscribeReject},
    };
    use serde_json::json;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";
    const OTHER_DOMAIN_ID: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const VALID_HASH: &str = "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const NOW: &str = "2026-05-26T12:30:00Z";

    #[derive(Default)]
    struct StaticGetClient {
        frame: Option<Vec<u8>>,
        error: Option<PathClientError>,
        request: Option<GetRequest>,
    }

    impl GetClient for StaticGetClient {
        fn get(
            &mut self,
            _peer_id: PeerId,
            request: GetRequest,
        ) -> Result<Vec<u8>, PathClientError> {
            self.request = Some(request);
            if let Some(error) = self.error.clone() {
                Err(error)
            } else {
                Ok(self.frame.clone().expect("test frame"))
            }
        }
    }

    #[derive(Default)]
    struct StaticSubscribeClient {
        frame: Option<Vec<u8>>,
        error: Option<PathClientError>,
        request: Option<SubscribeRequest>,
    }

    impl SubscribeClient for StaticSubscribeClient {
        fn subscribe(
            &mut self,
            _peer_id: PeerId,
            request: SubscribeRequest,
        ) -> Result<Vec<u8>, PathClientError> {
            self.request = Some(request);
            if let Some(error) = self.error.clone() {
                Err(error)
            } else {
                Ok(self.frame.clone().expect("test frame"))
            }
        }
    }

    fn test_config() -> AukiP2pConfig {
        AukiP2pConfig::development()
    }

    fn test_relationship() -> PeerRelationship {
        let wallet = Wallet::from_seed(vec![101; 32]).expect("32-byte seed");
        let identity = crate::LocalPeerIdentity::from_wallet(
            wallet,
            "2026-05-26T12:00:00Z",
            Some("paths-test"),
        )
        .expect("identity");
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.state = PeerRelationshipState::Ready;
        relationship.connected = true;
        relationship.authorized = true;
        relationship.accepted_served_domains = vec![DOMAIN_ID.to_owned()];
        relationship.offer_catalog_state = OfferCatalogLoadState::Loaded;
        relationship
    }

    fn offer_report() -> OfferLoadReport {
        OfferLoadReport {
            peer_id: test_relationship().peer_id,
            offers: vec![LoadedRemoteOffer {
                offer: offer("camera-main", "auki.frame"),
                usable: true,
                unusable_reason: None,
            }],
            diagnostics: Vec::new(),
            generated_at: Some("2026-05-26T12:00:01Z".to_owned()),
        }
    }

    fn unusable_offer_report(code: &'static str) -> OfferLoadReport {
        OfferLoadReport {
            peer_id: test_relationship().peer_id,
            offers: vec![LoadedRemoteOffer {
                offer: offer("camera-main", "auki.frame"),
                usable: false,
                unusable_reason: Some(code),
            }],
            diagnostics: Vec::new(),
            generated_at: None,
        }
    }

    fn offer(offer_id: &str, payload_type: &str) -> Offer {
        let reference = RegistryReference::create("clock", "clock", "clock-main", VALID_HASH, None)
            .expect("registry ref");
        Offer::create(
            offer_id,
            DOMAIN_ID,
            "sensor.frame",
            OfferStatus::Available,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe],
            PayloadDescriptor::create(payload_type),
            vec![reference],
        )
        .expect("offer")
    }

    fn message_value(domain_id: &str, offer_id: &str, payload_type: &str, sequence: u64) -> Value {
        json!({
            "type": auki_protocol::v1::message::SPATIAL_MESSAGE_TYPE,
            "domain_id": domain_id,
            "offer_id": offer_id,
            "payload": {
                "type": payload_type,
                "bytes": "AQID",
                "json": {"ok": true},
            },
            "sequence": sequence.to_string(),
            "generated_at": "2026-05-26T12:00:00Z",
        })
    }

    fn message(
        domain_id: &str,
        offer_id: &str,
        payload_type: &str,
        sequence: u64,
    ) -> SpatialMessage {
        SpatialMessage::from_value(message_value(domain_id, offer_id, payload_type, sequence))
            .expect("message")
    }

    fn frame(value: &Value, max: u64) -> Vec<u8> {
        encode_json_frame(value, max).expect("frame")
    }

    fn context<'a>(config: &'a AukiP2pConfig) -> PathContext<'a> {
        PathContext::new(config, NOW)
    }

    #[test]
    fn get_builds_high_level_request_and_tracks_success() {
        let config = test_config();
        let mut relationship = test_relationship();
        let response = GetResponse::success(message(DOMAIN_ID, "camera-main", "auki.frame", 7));
        let mut client = StaticGetClient {
            frame: Some(frame(
                response.value(),
                config.limits.get_response_frame_body_bytes,
            )),
            ..StaticGetClient::default()
        };
        let mut input = GetInput::new(DOMAIN_ID, "camera-main");
        input.params = Some(json!({"latest": true}));
        input.accepted_payload_types = vec!["auki.frame".to_owned()];
        input.max_payload_bytes = Some(512);

        let outcome = get(
            &mut relationship,
            &offer_report(),
            &mut client,
            input,
            context(&config),
        )
        .expect("get succeeds");

        assert_eq!(outcome.message.sequence, Some(7));
        let request = client.request.expect("get request");
        assert_eq!(request.domain_id, DOMAIN_ID);
        assert_eq!(request.offer_id, "camera-main");
        assert_eq!(request.params, Some(json!({"latest": true})));
        assert_eq!(request.max_payload_bytes, Some(512));
        assert_eq!(relationship.paths[0].state.as_deref(), Some(GET_SUCCEEDED));
        assert_eq!(
            relationship.paths[0].path_type.as_deref(),
            Some(GET_PATH_TYPE)
        );
    }

    #[test]
    fn get_uses_lower_payload_limit_and_validates_selected_payload() {
        let mut config = test_config();
        config.limits.get_response_frame_body_bytes = 32;
        let mut relationship = test_relationship();
        let response = GetResponse::success(message(DOMAIN_ID, "camera-main", "auki.frame", 1));
        let mut client = StaticGetClient {
            frame: Some(frame(response.value(), 4096)),
            ..StaticGetClient::default()
        };
        let mut input = GetInput::new(DOMAIN_ID, "camera-main");
        input.max_payload_bytes = Some(1024);

        let error = get(
            &mut relationship,
            &offer_report(),
            &mut client,
            input,
            context(&config),
        )
        .expect_err("frame body exceeds local limit");

        assert!(matches!(
            error,
            PathOrchestrationError::GetFrame(FrameError::BodyTooLarge { .. })
        ));

        let config = test_config();
        let mut relationship = test_relationship();
        let response = GetResponse::success(message(DOMAIN_ID, "camera-main", "other.payload", 1));
        let mut client = StaticGetClient {
            frame: Some(frame(
                response.value(),
                config.limits.get_response_frame_body_bytes,
            )),
            ..StaticGetClient::default()
        };

        let error = get(
            &mut relationship,
            &offer_report(),
            &mut client,
            GetInput::new(DOMAIN_ID, "camera-main"),
            context(&config),
        )
        .expect_err("payload mismatch");

        assert!(matches!(
            error,
            PathOrchestrationError::GetResponse(GetResponseError::InvalidMessage(_))
        ));
        assert_eq!(relationship.paths[0].state.as_deref(), Some(PATH_FAILED));
    }

    #[test]
    fn get_records_unknown_unusable_and_remote_failures_without_trusting_retryable() {
        let config = test_config();
        let mut relationship = test_relationship();
        let mut client = StaticGetClient::default();

        let error = get(
            &mut relationship,
            &offer_report(),
            &mut client,
            GetInput::new(DOMAIN_ID, "missing"),
            context(&config),
        )
        .expect_err("unknown offer");
        assert_eq!(error.failure_code(), error::OFFER_UNKNOWN_OFFER);

        let error = get(
            &mut relationship,
            &unusable_offer_report(error::OFFER_STALE),
            &mut client,
            GetInput::new(DOMAIN_ID, "camera-main"),
            context(&config),
        )
        .expect_err("unusable offer");
        assert_eq!(error.failure_code(), error::OFFER_STALE);

        let remote_error = json!({
            "type": auki_protocol::v1::get::GET_RESPONSE_TYPE,
            "error": {
                "code": error::OFFER_UNKNOWN_OFFER,
                "retryable": true,
                "message": "remote says retry",
            },
        });
        let mut client = StaticGetClient {
            frame: Some(frame(
                &remote_error,
                config.limits.get_response_frame_body_bytes,
            )),
            ..StaticGetClient::default()
        };
        let error = get(
            &mut relationship,
            &offer_report(),
            &mut client,
            GetInput::new(DOMAIN_ID, "camera-main"),
            context(&config),
        )
        .expect_err("remote get rejection");

        assert_eq!(error.failure_code(), error::OFFER_UNKNOWN_OFFER);
        assert_eq!(
            relationship.last_failures.last().unwrap().retryable,
            Some(false)
        );
        assert!(
            !relationship
                .last_failures
                .last()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .contains("remote says retry")
        );
    }

    #[test]
    fn subscribe_returns_handle_and_tracks_active_path() {
        let config = test_config();
        let mut relationship = test_relationship();
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            Vec::new(),
            Some(1),
            Some("2026-05-26T12:00:00Z".to_owned()),
            None,
        )
        .expect("accept");
        let mut client = StaticSubscribeClient {
            frame: Some(frame(
                accept.value(),
                config.limits.subscribe_message_frame_body_bytes,
            )),
            ..StaticSubscribeClient::default()
        };
        let mut input = SubscribeInput::new(DOMAIN_ID, "camera-main");
        input.max_message_bytes = Some(2048);

        let handle = subscribe(
            &mut relationship,
            &offer_report(),
            &mut client,
            input,
            context(&config),
        )
        .expect("subscribe accepted");

        assert_eq!(handle.payload_type(), "auki.frame");
        assert_eq!(handle.last_sequence(), None);
        assert_eq!(
            relationship.paths[0].state.as_deref(),
            Some(SUBSCRIBE_ACTIVE)
        );
        assert_eq!(
            client.request.expect("subscribe request").max_message_bytes,
            Some(2048)
        );
    }

    #[test]
    fn subscribe_uses_lower_message_limit_and_validates_accept_payload() {
        let mut config = test_config();
        config.limits.subscribe_message_frame_body_bytes = 512;
        let mut relationship = test_relationship();
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            Vec::new(),
            None,
            None,
            None,
        )
        .expect("accept");
        let mut client = StaticSubscribeClient {
            frame: Some(frame(accept.value(), 4096)),
            ..StaticSubscribeClient::default()
        };
        let mut input = SubscribeInput::new(DOMAIN_ID, "camera-main");
        input.max_message_bytes = Some(1024);

        subscribe(
            &mut relationship,
            &offer_report(),
            &mut client,
            input,
            context(&config),
        )
        .expect("subscribe accepted");

        assert_eq!(
            client.request.expect("subscribe request").max_message_bytes,
            Some(512)
        );

        let config = test_config();
        let mut relationship = test_relationship();
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("other.payload"),
            Vec::new(),
            None,
            None,
            None,
        )
        .expect("accept");
        let mut client = StaticSubscribeClient {
            frame: Some(frame(
                accept.value(),
                config.limits.subscribe_message_frame_body_bytes,
            )),
            ..StaticSubscribeClient::default()
        };

        let error = subscribe(
            &mut relationship,
            &offer_report(),
            &mut client,
            SubscribeInput::new(DOMAIN_ID, "camera-main"),
            context(&config),
        )
        .expect_err("accept payload not requested");

        assert!(matches!(
            error,
            PathOrchestrationError::SubscribeAccept(
                SubscribeAcceptError::PayloadTypeNotAccepted { .. }
            )
        ));
    }

    #[test]
    fn subscribe_reject_records_structured_failure_without_remote_retryable() {
        let config = test_config();
        let mut relationship = test_relationship();
        let reject = SubscribeReject::create(
            ErrorObject::from_value(json!({
                "code": error::OFFER_UNKNOWN_OFFER,
                "retryable": true,
                "message": "remote retry hint",
            }))
            .expect("error object"),
        );
        let mut client = StaticSubscribeClient {
            frame: Some(frame(
                reject.value(),
                config.limits.subscribe_message_frame_body_bytes,
            )),
            ..StaticSubscribeClient::default()
        };

        let error = subscribe(
            &mut relationship,
            &offer_report(),
            &mut client,
            SubscribeInput::new(DOMAIN_ID, "camera-main"),
            context(&config),
        )
        .expect_err("subscribe rejected");

        assert_eq!(error.failure_code(), error::OFFER_UNKNOWN_OFFER);
        assert_eq!(
            relationship.last_failures.last().unwrap().retryable,
            Some(false)
        );
        assert!(
            !relationship
                .last_failures
                .last()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .contains("remote retry hint")
        );
    }

    #[test]
    fn subscribe_data_uses_actual_frame_body_length() {
        let mut config = test_config();
        config.limits.subscribe_message_frame_body_bytes = 4096;
        let mut relationship = test_relationship();
        let mut handle = subscription_handle(Some(1), Some(8));
        let data = message_value(DOMAIN_ID, "camera-main", "auki.frame", 1);
        let data_frame = frame(&data, config.limits.subscribe_message_frame_body_bytes);

        let error = accept_subscribe_data_frame(
            &mut relationship,
            &mut handle,
            &data_frame,
            context(&config),
        )
        .expect_err("actual frame body exceeds effective subscription limit");

        assert!(matches!(
            error,
            PathOrchestrationError::SubscribeData(SubscribeDataError::MessageTooLarge { .. })
        ));
        assert_eq!(
            relationship.last_failures.last().unwrap().code,
            error::MESSAGE_PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn subscribe_data_tracks_sequence_gaps_without_failing_message() {
        let config = test_config();
        let mut relationship = test_relationship();
        let mut handle = subscription_handle(Some(1), Some(4096));
        let first = frame(
            &message_value(DOMAIN_ID, "camera-main", "auki.frame", 1),
            config.limits.subscribe_message_frame_body_bytes,
        );
        accept_subscribe_data_frame(
            &mut relationship,
            &mut handle,
            &first,
            PathContext::new(&config, "2026-05-26T12:31:00Z"),
        )
        .expect("first message");
        let third = frame(
            &message_value(DOMAIN_ID, "camera-main", "auki.frame", 3),
            config.limits.subscribe_message_frame_body_bytes,
        );

        let message = accept_subscribe_data_frame(
            &mut relationship,
            &mut handle,
            &third,
            PathContext::new(&config, "2026-05-26T12:32:00Z"),
        )
        .expect("gap is diagnostic only");

        assert_eq!(message.sequence, Some(3));
        assert_eq!(handle.sequence_gap_count(), 1);
        assert_eq!(
            relationship.last_failures.last().unwrap().code,
            error::MESSAGE_SEQUENCE_GAP
        );
        assert_eq!(relationship.paths[0].sequence_gap_count, 1);
        assert_eq!(relationship.paths[0].last_sequence, Some(3));
        assert_eq!(relationship.paths[0].started_at.as_deref(), Some(NOW));
        assert_eq!(
            relationship.paths[0].last_message_at.as_deref(),
            Some("2026-05-26T12:32:00Z")
        );
        assert_eq!(
            relationship.paths[0]
                .last_envelope_failure
                .as_ref()
                .unwrap()
                .code,
            error::MESSAGE_SEQUENCE_GAP
        );
    }

    #[test]
    fn subscribe_end_tracks_ended_and_failed_states() {
        let config = test_config();
        let mut relationship = test_relationship();
        let mut handle = subscription_handle(Some(1), Some(4096));
        let end = auki_protocol::v1::subscribe::SubscribeEnd::create(
            DOMAIN_ID,
            "camera-main",
            SubscribeEndReason::Complete,
            None,
            None,
            None,
        )
        .expect("end");
        let end_frame = frame(
            end.value(),
            config.limits.subscribe_message_frame_body_bytes,
        );

        end_subscription_from_frame(&mut relationship, &mut handle, &end_frame, context(&config))
            .expect("end");

        assert_eq!(
            relationship.paths[0].state.as_deref(),
            Some(SUBSCRIBE_ENDED)
        );

        let mut relationship = test_relationship();
        let mut handle = subscription_handle(Some(1), Some(4096));
        let end = auki_protocol::v1::subscribe::SubscribeEnd::create(
            OTHER_DOMAIN_ID,
            "camera-main",
            SubscribeEndReason::Complete,
            None,
            None,
            None,
        )
        .expect("end");
        let end_frame = frame(
            end.value(),
            config.limits.subscribe_message_frame_body_bytes,
        );

        let error = end_subscription_from_frame(
            &mut relationship,
            &mut handle,
            &end_frame,
            context(&config),
        )
        .expect_err("end path mismatch");

        assert!(matches!(
            error,
            PathOrchestrationError::SubscribeEnd(SubscribeEndError::PathMismatch { .. })
        ));
        assert_eq!(relationship.paths[0].state.as_deref(), Some(PATH_FAILED));
    }

    fn subscription_handle(
        initial_sequence: Option<u64>,
        max_message_bytes: Option<u64>,
    ) -> SubscriptionHandle {
        let request = SubscribeRequest::create(
            DOMAIN_ID,
            "camera-main",
            None,
            vec!["auki.frame".to_owned()],
            max_message_bytes,
        )
        .expect("request");
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            Vec::new(),
            initial_sequence,
            None,
            None,
        )
        .expect("accept");
        SubscriptionHandle {
            path_id: "subscription-test".to_owned(),
            peer_id: test_relationship().peer_id,
            request,
            accept,
            max_message_bytes,
            started_at: NOW.to_owned(),
            last_message_at: None,
            next_expected_sequence: initial_sequence,
            last_sequence: None,
            sequence_gap_count: 0,
            last_envelope_failure: None,
            last_payload_failure: None,
            last_failure: None,
        }
    }
}
