//! `fount-daemon`: Asynchronous IPC search daemon as part of Fount.
//!
//! Bridges the synchronous `fount-core` search engine via a dedicated OS actor thread
//! to non-blocking Tokio IPC transports.

pub mod engine_actor;
pub mod protocol;

pub use engine_actor::{EngineCommand, EngineError, EngineHandle};
pub use protocol::{ClientMessage, DaemonMessage, SearchResultWire};
