//! Click-to-login for the agent runtimes, modeled on IgnitionRAG's flow:
//! spawn the provider's login command, scrape the OAuth URL (and, for codex,
//! the device code) from its output, and read the exit code — 0 means
//! authenticated. Claude takes the authorization code on stdin; codex polls
//! and exits on its own. Sessions live in memory only: single-user, and a
//! restart just means start over.
//!
//! State machine: Starting → WaitingForInput → Validating → Authenticated |
//! Failed, plus Expired past the TTL. The process tree dies with the session.

use crate::config::AgentCommand;
use latoile_core::ports::PortError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::{Child, ChildStdin};

/// The login challenge lives 15 minutes, then the tree is killed.
pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);

/// The three commands of one provider's auth lifecycle.
#[derive(Debug, Clone)]
pub struct ProviderCommands {
    pub login: AgentCommand,
    pub status: AgentCommand,
    pub logout: AgentCommand,
}

/// Whether a provider is authenticated, per its own CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub authenticated: bool,
    /// Account email, login method… whatever the CLI says, if anything.
    pub detail: Option<String>,
}

/// Which agent runtime is being logged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthProvider {
    Claude,
    Codex,
}

impl AuthProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The wire name, from the start request.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
    /// The three lifecycle commands. Status truth comes from the CLI
    /// itself, not the credential file: on macOS Claude stores credentials
    /// in the Keychain, so `~/.claude/.credentials.json` may not exist for
    /// a logged-in user.
    fn commands(&self) -> ProviderCommands {
        match self {
            Self::Claude => ProviderCommands {
                login: AgentCommand::new("claude").args(["auth", "login", "--claudeai"]),
                status: AgentCommand::new("claude").args(["auth", "status"]),
                logout: AgentCommand::new("claude").args(["auth", "logout"]),
            },
            Self::Codex => ProviderCommands {
                login: AgentCommand::new("codex").args(["login", "--device-auth"]),
                status: AgentCommand::new("codex").args(["login", "status"]),
                logout: AgentCommand::new("codex").args(["logout"]),
            },
        }
    }

    /// Claude reads the authorization code on stdin; Codex polls and exits
    /// on its own — no input channel.
    fn input_required(&self) -> bool {
        matches!(self, Self::Claude)
    }

    /// Hosts whose https URLs count as the OAuth challenge.
    fn url_hosts(&self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["claude.com"],
            Self::Codex => &["openai.com", "chatgpt.com"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStatus {
    Starting,
    WaitingForInput,
    Validating,
    Authenticated,
    Failed,
    Expired,
}

impl AuthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::WaitingForInput => "waiting_for_input",
            Self::Validating => "validating",
            Self::Authenticated => "authenticated",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Authenticated | Self::Failed | Self::Expired)
    }
}

/// What failed while managing a login session.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("spawning the login command failed: {0}")]
    Spawn(String),
    #[error("unknown auth session")]
    Unknown,
    #[error("the session is not waiting for a code")]
    NotWaiting,
    #[error("this provider does not take a code — it confirms itself")]
    InputNotRequired,
}

impl From<AuthError> for PortError {
    fn from(e: AuthError) -> Self {
        PortError(e.to_string())
    }
}

/// A session as the outside world sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSessionView {
    pub id: String,
    pub provider: AuthProvider,
    pub status: AuthStatus,
    pub url: Option<String>,
    /// The device code (codex): shown to the user, never sent anywhere.
    pub user_code: Option<String>,
    /// False for codex — no paste field.
    pub input_required: bool,
    /// The last lines of CLI output: the honest fallback when scraping
    /// finds nothing recognizable.
    pub hint: Option<String>,
    pub error: Option<String>,
}

struct SessionEntry {
    provider: AuthProvider,
    status: AuthStatus,
    url: Option<String>,
    user_code: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    stdin: Option<ChildStdin>,
    child: Child,
    deadline: Instant,
}

impl SessionEntry {
    fn view(&self, id: &str) -> AuthSessionView {
        AuthSessionView {
            id: id.to_string(),
            provider: self.provider,
            status: self.status,
            url: self.url.clone(),
            user_code: self.user_code.clone(),
            input_required: self.provider.input_required(),
            hint: self.hint.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AgentAuthManager {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    commands: HashMap<AuthProvider, ProviderCommands>,
    ttl: Duration,
}

impl AgentAuthManager {
    /// The real commands, 15-minute TTL.
    pub fn production() -> Self {
        Self::new(DEFAULT_TTL)
    }

    /// Default commands, custom TTL (tests).
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            commands: HashMap::from([
                (AuthProvider::Claude, AuthProvider::Claude.commands()),
                (AuthProvider::Codex, AuthProvider::Codex.commands()),
            ]),
            ttl,
        }
    }

    /// Override one provider's login command — the test seam.
    pub fn with_command(mut self, provider: AuthProvider, command: AgentCommand) -> Self {
        self.commands
            .get_mut(&provider)
            .expect("every provider has commands")
            .login = command;
        self
    }

    /// Override the whole command set (status/disconnect tests).
    pub fn with_commands(mut self, provider: AuthProvider, commands: ProviderCommands) -> Self {
        self.commands.insert(provider, commands);
        self
    }
}
mod lifecycle;
mod scrape;
mod session;

#[cfg(test)]
mod tests;
