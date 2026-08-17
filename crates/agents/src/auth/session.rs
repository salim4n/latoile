//! One interactive login session: supervised process, output scraping,
//! state refresh, code submission, expiry, and process-tree cleanup.

use super::scrape::{find_device_code, find_oauth_url, last_lines, strip_ansi};
use super::{AgentAuthManager, AuthError, AuthProvider, AuthSessionView, AuthStatus, SessionEntry};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

/// Retain enough CLI output for a useful fallback without letting a noisy
/// or wedged child grow the in-memory session forever.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

impl AgentAuthManager {
    /// Spawn the provider's login command and start scraping. Starting
    /// until the URL lands — the UI polls.
    pub async fn start(&self, provider: AuthProvider) -> Result<AuthSessionView, AuthError> {
        let command = &self
            .commands
            .get(&provider)
            .expect("every provider has commands")
            .login;
        let (program, args, env) = command.as_parts();
        let mut cmd = Command::new(program);
        cmd.args(args)
            .envs(env.iter().cloned())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // The login CLI may re-exec (codex's wrapper → vendored binary);
        // its own group lets us kill the whole tree.
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| AuthError::Spawn(format!("{command}: {e}")))?;

        let id = ulid::Ulid::new().to_string();
        let entry = SessionEntry {
            provider,
            status: AuthStatus::Starting,
            url: None,
            user_code: None,
            hint: None,
            error: None,
            stdin: child.stdin.take(),
            child,
            deadline: Instant::now() + self.ttl,
        };
        self.sessions
            .lock()
            .expect("auth sessions poisoned")
            .insert(id.clone(), entry);

        // The entry owns the child; scrape both output streams.
        let (stdout, stderr) = {
            let mut sessions = self.sessions.lock().expect("auth sessions poisoned");
            let entry = sessions.get_mut(&id).expect("just inserted");
            (
                entry.child.stdout.take().expect("stdout piped"),
                entry.child.stderr.take().expect("stderr piped"),
            )
        };
        self.scrape(provider, id.clone(), stdout);
        self.scrape(provider, id.clone(), stderr);

        Ok(self.status(&id).expect("just inserted"))
    }

    /// Wire one output stream into the scraper: URL, device code, and the
    /// output tail (the hint the UI shows when nothing matches).
    fn scrape(
        &self,
        provider: AuthProvider,
        id: String,
        mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    ) {
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > MAX_OUTPUT_BYTES {
                            buf.drain(..buf.len() - MAX_OUTPUT_BYTES);
                        }
                        let plain = strip_ansi(&String::from_utf8_lossy(&buf));
                        let mut sessions = sessions.lock().expect("auth sessions poisoned");
                        if let Some(entry) = sessions.get_mut(&id) {
                            entry.hint = Some(last_lines(&plain, 5));
                            if entry.url.is_none() {
                                entry.url = find_oauth_url(&plain, provider.url_hosts());
                            }
                            if !provider.input_required() && entry.user_code.is_none() {
                                entry.user_code = find_device_code(&plain);
                            }
                            // Claude needs only the URL; Codex shows URL and
                            // device code together, so both before waiting.
                            let ready = entry.url.is_some()
                                && (provider.input_required() || entry.user_code.is_some());
                            if ready && entry.status == AuthStatus::Starting {
                                entry.status = AuthStatus::WaitingForInput;
                            }
                        }
                    }
                }
            }
        });
    }

    /// The current state. Poll-based: exit detection (try_wait) and expiry
    /// happen here, on read — no watcher task.
    pub fn status(&self, id: &str) -> Option<AuthSessionView> {
        let mut sessions = self.sessions.lock().expect("auth sessions poisoned");
        let entry = sessions.get_mut(id)?;
        refresh(entry);
        Some(entry.view(id))
    }

    /// Write the authorization code to the child's stdin. The stdin handle
    /// leaves the map for the write: holding a std MutexGuard across an
    /// await would make the future !Send (and axum requires Send).
    pub async fn submit_code(&self, id: &str, code: &str) -> Result<AuthSessionView, AuthError> {
        let code = code.trim();
        if code.is_empty() || code.len() > 4096 {
            return Err(AuthError::NotWaiting);
        }
        let mut stdin = {
            let mut sessions = self.sessions.lock().expect("auth sessions poisoned");
            let entry = sessions.get_mut(id).ok_or(AuthError::Unknown)?;
            refresh(entry);
            if !entry.provider.input_required() {
                return Err(AuthError::InputNotRequired);
            }
            if entry.status != AuthStatus::WaitingForInput {
                return Err(AuthError::NotWaiting);
            }
            entry.stdin.take().ok_or(AuthError::NotWaiting)?
        };

        let write = async {
            stdin.write_all(format!("{code}\n").as_bytes()).await?;
            stdin.flush().await
        }
        .await;

        let mut sessions = self.sessions.lock().expect("auth sessions poisoned");
        let entry = sessions.get_mut(id).ok_or(AuthError::Unknown)?;
        entry.stdin = Some(stdin);
        match write {
            Ok(()) => {
                entry.status = AuthStatus::Validating;
                Ok(entry.view(id))
            }
            Err(e) => {
                entry.status = AuthStatus::Failed;
                entry.error = Some("lost the login process".into());
                Err(AuthError::Spawn(e.to_string()))
            }
        }
    }
}

/// Kill the login process and anything it re-exec'd into.
#[cfg(unix)]
fn kill_process_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        // SAFETY: killpg on the group we created at spawn; a dead group
        // just errors out and the direct kill below still runs.
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
}

#[cfg(not(unix))]
fn kill_process_tree(child: &mut Child) {
    let _ = child.start_kill();
}

impl Drop for SessionEntry {
    fn drop(&mut self) {
        if !self.status.is_terminal() {
            kill_process_tree(&mut self.child);
        }
    }
}

/// Exit and expiry detection, folded into a status read.
fn refresh(entry: &mut SessionEntry) {
    if entry.status.is_terminal() {
        return;
    }
    match entry.child.try_wait() {
        Ok(Some(status)) => {
            entry.status = if status.success() {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Failed
            };
            entry.error = if status.success() {
                None
            } else {
                Some(format!("login exited with {status}"))
            };
        }
        Ok(None) => {
            if Instant::now() >= entry.deadline {
                entry.status = AuthStatus::Expired;
                entry.error = Some("the login challenge expired".into());
                kill_process_tree(&mut entry.child);
            }
        }
        Err(_) => {
            entry.status = AuthStatus::Failed;
            entry.error = Some("lost the login process".into());
        }
    }
}
