//! trusty-daemon library — daemon loop and IPC server.
//!
//! The daemon runs as a background process, polling Gmail on a configurable
//! interval and serving an IPC socket that the CLI uses for control messages.

pub mod dispatcher;
pub mod handlers;
pub mod ipc;
pub mod loop_runner;

pub use dispatcher::EventDispatcher;
pub use loop_runner::DaemonLoop;
