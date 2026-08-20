//! The `PreviewSupervisor` port. One registry entry per project — the
//! partial-unique invariant lives in the database, but keying here by
//! project means a recycled preview id never leaks a process either.
//!
//! `ensure` is start-or-recycle: an existing server for the project is
//! killed first, then the command runs again. A refresh from `stale` (the
//! use case's path) keeps the port when it is still free, so the URL the UI
//! already shows survives the restart.

use crate::alloc::PortAllocator;
use crate::command;
use crate::logs::LogRing;
use crate::process::DevServer;
use latoile_core::ids::PreviewId;
use latoile_core::ports::{PortResult, PreviewSupervisor};
use latoile_core::Preview;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct SupervisorConfig {
    /// First port offered to previews (the spec's example serves on 4100).
    pub base_port: u16,
    /// How long a dev server gets to start listening before it is killed.
    pub readiness: Duration,
    /// Log lines kept per preview.
    pub log_capacity: usize,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            base_port: crate::alloc::DEFAULT_BASE_PORT,
            readiness: Duration::from_secs(30),
            log_capacity: crate::logs::DEFAULT_CAPACITY,
        }
    }
}

struct Entry {
    preview_id: PreviewId,
    server: DevServer,
}

pub struct Supervisor {
    config: SupervisorConfig,
    servers: std::sync::Arc<Mutex<HashMap<String, Entry>>>,
    allocator: std::sync::Arc<Mutex<PortAllocator>>,
}

impl Clone for Supervisor {
    /// Shared ownership of one registry: the server hands clones to handlers
    /// and they must all supervise the SAME dev servers.
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            servers: self.servers.clone(),
            allocator: self.allocator.clone(),
        }
    }
}

impl Supervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            servers: std::sync::Arc::new(Mutex::new(HashMap::new())),
            allocator: std::sync::Arc::new(Mutex::new(PortAllocator::default())),
        }
    }

    /// The buffered dev-server output, oldest first. Empty for an unknown or
    /// stopped preview — the server crate streams this to the UI.
    pub async fn logs(&self, preview: &PreviewId) -> Vec<String> {
        let servers = self.servers.lock().await;
        servers
            .values()
            .find(|e| e.preview_id == *preview)
            .map(|e| e.server.logs.snapshot())
            .unwrap_or_default()
    }

    /// Whether the project's dev server is still alive — the path a health
    /// loop uses to mark a preview `error` (PreviewState::fail) instead of
    /// discovering a corpse on the next click.
    pub async fn is_alive(&self, preview: &PreviewId) -> bool {
        let mut servers = self.servers.lock().await;
        servers
            .values_mut()
            .find(|e| e.preview_id == *preview)
            .map(|e| !e.server.has_exited())
            .unwrap_or(false)
    }

    async fn kill_project(&self, project: &str) {
        let entry = self.servers.lock().await.remove(project);
        if let Some(entry) = entry {
            let port = entry.server.port;
            entry.server.kill().await;
            self.allocator.lock().await.release(port);
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new(SupervisorConfig::default())
    }
}

impl PreviewSupervisor for Supervisor {
    async fn ensure(
        &self,
        preview: &Preview,
        dev_command: &str,
        working_dir: &str,
    ) -> PortResult<(u32, u16)> {
        let project = preview.project_id.as_str().to_string();
        // Start-or-recycle: whatever ran for this project goes first.
        self.kill_project(&project).await;

        let dev_command = command::resolve(dev_command, working_dir).await?;

        let preferred = (preview.port != 0).then_some(preview.port);
        let port = self
            .allocator
            .lock()
            .await
            .take_except(self.config.base_port, preferred)
            .await?;

        let ring = LogRing::new(self.config.log_capacity);
        let server =
            match DevServer::spawn(&dev_command, working_dir, port, ring, self.config.readiness)
                .await
            {
                Ok(server) => server,
                Err(e) => {
                    self.allocator.lock().await.release(port);
                    return Err(e.into());
                }
            };
        let pid = server.pid;
        self.servers.lock().await.insert(
            project,
            Entry {
                preview_id: preview.id.clone(),
                server,
            },
        );
        Ok((pid, port))
    }

    /// Unknown or already stopped is success: the wanted state is "not
    /// running".
    async fn stop(&self, preview: &Preview) -> PortResult<()> {
        self.kill_project(preview.project_id.as_str()).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod qa_regression_issue_004;
