//! Typed, network-independent Component dataflow primitives.
//!
//! Executable [`Component`]s own live typed interfaces. [`Observable`]s emit
//! timestamped values, [`Operable`]s accept typed instructions, and retained
//! [`RetainedProduct`]s are explicit data products rather than hidden queues.
//! A [`ComponentRuntime`] projects constructed live handles into a read-only
//! [`Catalog`]. Network transport is intentionally outside this crate.
//!
//! Port types make incompatible connections fail at compile time:
//!
//! ```compile_fail
//! use auki_components::{connect, ConnectionOptions, InputPort, OutputPort};
//!
//! struct CameraFrame;
//! struct AudioFrame;
//!
//! let camera = OutputPort::<CameraFrame>::new("camera.frames");
//! let microphone = InputPort::<AudioFrame>::new("microphone.frames", |_| {});
//! let _connection = connect(&camera, &microphone, ConnectionOptions::InlineEvery).unwrap();
//! ```
//!
//! Catalog mutation is unavailable to applications; entries originate from
//! live runtime handles:
//!
//! ```compile_fail
//! use auki_components::{ComponentManifest, ComponentRuntime};
//!
//! let runtime = ComponentRuntime::new("peer-a");
//! runtime.catalog().register_component(ComponentManifest {
//!     schema: "made-up/v1".into(), peer_id: "peer-a".into(),
//!     component_id: "not-live".into(), product_inputs: vec![],
//!     observables: vec![], operables: vec![],
//! });
//! ```

mod buffer;
mod camera;
mod component;
mod episode;
mod ports;
mod product;
mod pump;
mod runtime;

pub use buffer::{
    Buffer, BufferCursor, BufferError, BufferLimits, BufferRange, BufferReader, BufferReaderStats,
    BufferTimePolicy, CursorRead, CursorStart, DurationTimeBasis, Gap, SourceTimestampPolicy,
    connect_buffer,
};
pub use camera::{
    AppliedResolution, CameraBufferCapture, CameraBufferError, CameraComponent, CameraError,
    CameraProductBuffer, DriverReseeded, ReseedDriver, SetResolution, VideoFrame,
};
pub use component::{
    AudioLayout, AudioPayloadContract, AudioSampleFormat, CameraPayloadContract, Catalog,
    CatalogComponentEntry, CatalogError, CatalogOutputEntry, CatalogProductEntry,
    CatalogProductInputEntry, CatalogSnapshot, ComponentManifest, ComponentReference,
    ComponentRuntime, EverySelectedDelivery, Exposure, GaugePayloadContract, InMemoryTransport,
    Invocation, InvocationContext, InvocationError, InvocationHandle, InvocationOptions,
    InvocationOrdering, InvocationStatus, ManifestHash, Observable, ObservableContract,
    Observation, ObservationAccess, ObservationDelivery, ObservationEnd, ObservationEndReason,
    ObservationError, ObservationEvent, ObservationHandle, ObservationStats, ObservationStatus,
    Operable, OperableContract, OutputManifest, OutputReference, PayloadContract, ProductForm,
    ProductInputBindingManifest, ProductInputContract, ProductManifest, ProductReference,
    ProductState, SerializedInMemoryTransport, StructuredPayloadContract, TransportStats,
    manifest_hash, observation_input,
};
pub use episode::{Episode, EpisodeError, EpisodeState, connect_episode};
pub use ports::{
    ComponentError, Connection, ConnectionControl, ConnectionError, ConnectionOptions,
    ConnectionStats, Envelope, EveryFullPolicy, InputPort, OutputPort, PublishReport,
    SchedulerError, SharedDelivery, SharedDispatcher, SharedScheduler, SharedSchedulerStats,
    StaticConnection, connect, connect_shared,
};
pub use product::{
    BufferProductCapture, EpisodeProduct, EpisodeProductCapture, FiniteObservations,
    ProductAccessError, ProductCaptureError, ProductImportError, RetainedProduct, TimeRangeRequest,
};
pub use pump::{
    PumpError, PumpOptions, PumpStats, SinkFullPolicy, StreamPump, connect_direct_latest_pump,
};
pub use runtime::{
    Component, ComponentBuildError, ComponentSpec, ConfiguredBufferInput, ConfiguredObservable,
    ConfiguredObservableReplacement, ConfiguredObservableSpec, ContractType, PublishError,
};
