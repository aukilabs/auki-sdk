//! Network-independent experiment for typed, in-process SDK dataflow.
//!
//! The crate deliberately keeps executable Components separate from retained
//! data products. Components connect through typed ports; a [`Buffer`] is an
//! explicit owning subscriber; an [`Episode`] is a deliberately retained
//! interval; and a [`StreamPump`] follows a Buffer for one recipient.
//!
//! Port types make incompatible connections fail at compile time:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::{connect, ConnectionOptions, InputPort, OutputPort};
//!
//! struct CameraFrame;
//! struct AudioFrame;
//!
//! let camera = OutputPort::<CameraFrame>::new("camera.frames");
//! let microphone = InputPort::<AudioFrame>::new("microphone.frames", |_| {});
//! let _connection = connect(&camera, &microphone, ConnectionOptions::InlineEvery).unwrap();
//! ```

mod buffer;
mod chunk;
mod episode;
mod ports;
mod pump;

pub use buffer::{
    Buffer, BufferCursor, BufferError, BufferLimits, BufferRange, BufferReader, BufferReaderStats,
    CursorRead, CursorStart, Gap, connect_buffer,
};
pub use chunk::{Chunk, ChunkBuilder, ChunkBuilderConfig, ChunkBuilderError, ChunkBuilderStats};
pub use episode::{Episode, EpisodeError, EpisodeState, connect_episode};
pub use ports::{
    Connection, ConnectionError, ConnectionOptions, ConnectionStats, Envelope, EveryFullPolicy,
    InputPort, OutputPort, PublishReport, StaticConnection, connect,
};
pub use pump::{
    PumpError, PumpOptions, PumpStats, SinkFullPolicy, StreamPump, connect_direct_latest_pump,
};
