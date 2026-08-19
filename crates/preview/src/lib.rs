//! The preview adapter — dev-server supervision. One of the two crates
//! allowed to spawn processes (architecture contract §3).
//!
//! For each project it runs the project's own `dev_command` (`pnpm dev
//! --port $PORT`, …) through `sh -c` with an allocated `PORT`, waits until
//! the port answers, and keeps the output in a bounded ring the server will
//! stream to the UI. Stopping or dropping kills the whole process group —
//! no orphaned dev servers (spec §7 risk register).
//!
//! - [`alloc`] — port allocation (first free from a base, externally
//!   occupied ports skipped).
//! - [`logs`] — the bounded log ring.
//! - [`process`] — one dev-server process: spawn, readiness probe, kill.
//! - [`supervisor`] — the `PreviewSupervisor` port itself.
//!
//! Readiness is a TCP connect, not an HTTP request: no path or status
//! convention to agree on, and no HTTP client dependency. The reverse proxy
//! itself (spec §5.1) lives in the server crate — this crate only makes
//! dev servers exist.

mod alloc;
mod command;
mod error;
mod logs;
mod process;
mod supervisor;

pub use error::PreviewError;
pub use logs::LogRing;
pub use supervisor::{Supervisor, SupervisorConfig};
