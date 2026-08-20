//! Status and disconnect: the provider CLIs are the source of truth.
//!
//! - Claude: `claude auth status` prints JSON (`loggedIn`, `email`) — the
//!   credentials *file* is not reliable (macOS stores them in the Keychain).
//! - Codex: `codex login status` exits 0 and says so on stdout.
//! - Both have a real logout subcommand; we never touch credential files.

use super::{AgentAuthManager, AuthProvider, ProviderStatus};
use crate::config::AgentCommand;
use std::time::Duration;

/// A status/logout command answers instantly or not at all.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

impl AgentAuthManager {
    /// Whether the provider is authenticated right now. Never fails: a CLI
    /// that errors, times out, or says nothing parseable is "not
    /// authenticated" — the UI then offers the connect flow.
    pub async fn provider_status(&self, provider: AuthProvider) -> ProviderStatus {
        let command = &self
            .commands
            .get(&provider)
            .expect("every provider has commands")
            .status;
        match run_capture(command).await {
            Some(output) => parse_status(provider, &output),
            None => ProviderStatus {
                authenticated: false,
                detail: None,
            },
        }
    }

    /// Log out through the provider's own subcommand, then report the
    /// resulting status. A failed logout still returns the status — the UI
    /// shows what is true, not what we hoped.
    pub async fn disconnect(&self, provider: AuthProvider) -> ProviderStatus {
        let command = &self
            .commands
            .get(&provider)
            .expect("every provider has commands")
            .logout;
        let _ = run_capture(command).await;
        self.provider_status(provider).await
    }
}

/// Run a command, capture stdout (stderr appended), honor the timeout.
async fn run_capture(command: &AgentCommand) -> Option<String> {
    let (program, args, env) = command.as_parts();
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .envs(env.iter().cloned())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(format!("{}\n{text}", output.status))
}

/// Parse one provider's status output. `text` is `"{exit status}\n{output}"`.
fn parse_status(provider: AuthProvider, text: &str) -> ProviderStatus {
    match provider {
        AuthProvider::Claude => {
            // The JSON body, wherever it starts in the output.
            let json_start = text.find('{');
            let parsed =
                json_start.and_then(|i| serde_json::from_str::<serde_json::Value>(&text[i..]).ok());
            match parsed {
                Some(value) => {
                    let authenticated = value
                        .get("loggedIn")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let detail = value
                        .get("email")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    ProviderStatus {
                        authenticated,
                        detail,
                    }
                }
                None => ProviderStatus {
                    authenticated: false,
                    detail: None,
                },
            }
        }
        AuthProvider::Codex => {
            let ok = text
                .lines()
                .next()
                .is_some_and(|l| l.contains("exit status: 0"));
            ProviderStatus {
                authenticated: ok,
                detail: if ok {
                    text.lines()
                        .nth(1)
                        .filter(|l| !l.is_empty())
                        .map(str::to_string)
                } else {
                    None
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AgentAuthManager, ProviderCommands};
    use crate::config::AgentCommand;

    fn commands(status_script: &str, logout_script: &str) -> ProviderCommands {
        ProviderCommands {
            login: AgentCommand::new("true"),
            status: AgentCommand::new("sh").args(["-c", status_script]),
            logout: AgentCommand::new("sh").args(["-c", logout_script]),
        }
    }

    #[tokio::test]
    async fn claude_status_parses_the_json() {
        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Claude,
            commands(
                "printf '{\"loggedIn\": true, \"email\": \"moi@example.com\"}'",
                "true",
            ),
        );
        let status = mgr.provider_status(AuthProvider::Claude).await;
        assert!(status.authenticated);
        assert_eq!(status.detail.as_deref(), Some("moi@example.com"));
    }

    #[tokio::test]
    async fn claude_logged_out_json_is_not_authenticated() {
        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Claude,
            commands("printf '{\"loggedIn\": false}'", "true"),
        );
        assert!(
            !mgr.provider_status(AuthProvider::Claude)
                .await
                .authenticated
        );
    }

    #[tokio::test]
    async fn codex_status_is_the_exit_code() {
        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Codex,
            commands("echo 'Logged in using ChatGPT'", "true"),
        );
        let status = mgr.provider_status(AuthProvider::Codex).await;
        assert!(status.authenticated);
        assert_eq!(status.detail.as_deref(), Some("Logged in using ChatGPT"));

        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Codex,
            commands("echo 'Not logged in'; exit 1", "true"),
        );
        assert!(!mgr.provider_status(AuthProvider::Codex).await.authenticated);
    }

    #[tokio::test]
    async fn disconnect_runs_logout_then_reports_the_new_status() {
        // The fake logout "works" by flipping what status prints.
        let dir = tempfile::tempdir().unwrap();
        let flag = dir.path().join("logged-in");
        std::fs::write(&flag, "").unwrap();
        let status_script = format!(
            "if [ -f {} ]; then echo 'Logged in using ChatGPT'; else echo 'Not logged in'; exit 1; fi",
            flag.display()
        );
        let logout_script = format!("rm {}", flag.display());

        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Codex,
            commands(&status_script, &logout_script),
        );
        assert!(mgr.provider_status(AuthProvider::Codex).await.authenticated);
        let after = mgr.disconnect(AuthProvider::Codex).await;
        assert!(!after.authenticated, "logout ran and status reflects it");
    }

    #[tokio::test]
    async fn a_dead_cli_reports_not_authenticated() {
        let mgr = AgentAuthManager::new(crate::auth::DEFAULT_TTL).with_commands(
            AuthProvider::Claude,
            ProviderCommands {
                login: AgentCommand::new("true"),
                status: AgentCommand::new("definitely-not-a-binary-latoile"),
                logout: AgentCommand::new("true"),
            },
        );
        let status = mgr.provider_status(AuthProvider::Claude).await;
        assert!(!status.authenticated);
        assert!(status.detail.is_none());
    }
}
