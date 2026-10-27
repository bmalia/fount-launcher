//! `fount-daemon`: Asynchronous IPC search daemon for Fount.
//!
//! Bridges the synchronous `fount-core` search engine via a dedicated OS actor thread
//! to non-blocking Tokio IPC transports.

pub mod engine_actor;
pub mod ipc;
pub mod protocol;

pub use engine_actor::{EngineCommand, EngineError, EngineHandle};
pub use ipc::{
    UnixSocketClient, bind_unix_listener, default_socket_path, run_unix_socket_server,
    serve_unix_listener,
};
pub use protocol::{ClientMessage, DaemonMessage, SearchResultWire};
