//! One dev-server process. Spawned through `sh -c` so the project's
//! `dev_command` (`pnpm dev --port $PORT`, …) reads exactly like a shell
//! line, with `PORT` in the environment — the supervisor allocates the port,
//! the command never picks its own.
//!
//! Lifecycle: the child is its own process-group leader (unix), and dropping
//! or killing the [`DevServer`] SIGKILLs the whole group — a dev server that
//! forks (npm → node → vite) dies entirely, not just its top shell
//! (contract §3: no orphans).

use crate::error::PreviewError;
use crate::logs::LogRing;
use std::net::Ipv4Addr;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// How often the readiness probe retries.
const PROBE_INTERVAL: Duration = Duration::from_millis(100);

pub struct DevServer {
    child: Child,
    pub pid: u32,
    pub port: u16,
    pub logs: LogRing,
}

impl DevServer {
    /// Spawn and wait until the port answers a TCP connect, the process
    /// exits, or the budget runs out. On any failure the process is dead
    /// before the error returns.
    pub async fn spawn(
        dev_command: &str,
        port: u16,
        logs: LogRing,
        readiness: Duration,
    ) -> Result<Self, PreviewError> {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(dev_command)
            .env("PORT", port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|e| PreviewError::Spawn(format!("`{dev_command}`: {e}")))?;
        let pid = child.id().unwrap_or(0);

        if let Some(stdout) = child.stdout.take() {
            pipe_lines(stdout, logs.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pipe_lines(stderr, logs.clone());
        }

        if let Err(e) = wait_ready(&mut child, port, &logs, readiness).await {
            kill_group(pid);
            let _ = child.kill().await;
            return Err(e);
        }

        Ok(Self {
            child,
            pid,
            port,
            logs,
        })
    }

    /// Whether the process is gone. The supervisor polls this so a crashed
    /// dev server is noticed as `error`, not discovered as a hang.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Kill the process group and reap the direct child.
    pub async fn kill(mut self) {
        if !self.has_exited() {
            kill_group(self.pid);
        }
        let _ = self.child.kill().await;
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        // A drop without an explicit kill (a future cancelled mid-await)
        // still takes the whole group down.
        if !self.has_exited() {
            kill_group(self.pid);
        }
        let _ = self.child.start_kill();
    }
}

/// `process_group(0)` made the child a group leader whose id is its pid, so
/// the group answers to the pid we already know.
#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: killpg on a group we created; if the group is gone the call
    // simply fails and the fallback child kill still runs.
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

/// Every line of stdout and stderr lands in the ring, tagged neither way —
/// dev servers mix both streams freely and the order matters more than the
/// channel.
fn pipe_lines(reader: impl AsyncRead + Unpin + Send + 'static, ring: LogRing) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            ring.push(line);
        }
    });
}

/// Ready = something accepts a TCP connection on the port. Chosen over an
/// HTTP probe: every dev server LaToile previews speaks HTTP, but TCP needs
/// no path, no status-code convention, and no HTTP client dependency — and
/// it is the same signal the reverse proxy will rely on.
async fn wait_ready(
    child: &mut Child,
    port: u16,
    logs: &LogRing,
    budget: Duration,
) -> Result<(), PreviewError> {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Err(PreviewError::Exited(logs.tail(5))),
            Ok(None) => {}
            Err(e) => return Err(PreviewError::Spawn(e.to_string())),
        }
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(PreviewError::NotReady(logs.tail(5)));
        }
        tokio::time::sleep(PROBE_INTERVAL).await;
    }
}
