//! Which binary speaks ACP for which role, and how long we wait before
//! killing it.
//!
//! LaToile never handles the agents' API keys: Claude Code and Codex carry
//! their own auth from the user's machine (the CLI's own login). The D9 token
//! rule is about LaToile's HTTP API, not these subprocesses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// An agent subprocess invocation: program, arguments, extra environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl AgentCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// What the process spawner consumes.
    pub fn as_parts(&self) -> (&str, &[String], &[(String, String)]) {
        (&self.program, &self.args, &self.env)
    }
}

impl std::fmt::Display for AgentCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// Time budgets. Exceeding one kills the process — a hung agent is never
/// left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentTimeouts {
    /// `initialize` + `session/new`. A healthy local binary answers in
    /// seconds.
    pub handshake: Duration,
    /// One prompt turn. Coding runs are slow; this is the outer bound, not
    /// the expectation.
    pub prompt: Duration,
    /// Maximum time an ACP tool call may wait for the owner. Expiry is a
    /// refusal and is journaled by the supervision loop.
    pub permission: Duration,
}

impl Default for AgentTimeouts {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(30),
            prompt: Duration::from_secs(30 * 60),
            permission: Duration::from_secs(15 * 60),
        }
    }
}

/// The channel's static configuration.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// One command per role id (`manager`, `backend`, …); roles not listed
    /// fall back to `default_command`. Defaults below are the maintained ACP
    /// adapters: `@zed-industries/claude-agent-acp` (Claude Code, supersedes
    /// the deprecated `claude-code-acp`) and `@agentclientprotocol/codex-acp`
    /// (Codex CLI). Both must be on PATH — e.g. via `npm install -g`.
    pub commands: HashMap<String, AgentCommand>,
    pub default_command: AgentCommand,
    /// Where role skill preambles live (`<dir>/<skill>/SKILL.md`).
    pub skills_dir: PathBuf,
    pub timeouts: AgentTimeouts,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            commands: HashMap::new(),
            default_command: AgentCommand::new("claude-agent-acp"),
            skills_dir: PathBuf::from("skills"),
            timeouts: AgentTimeouts::default(),
        }
    }
}

impl ChannelConfig {
    /// The command a role runs under. Per-role overrides first, then the
    /// default. A `codex` entry would map to `AgentCommand::new("codex-acp")`.
    pub fn command_for(&self, role: &str) -> &AgentCommand {
        self.commands.get(role).unwrap_or(&self.default_command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_fall_back_to_the_default_command() {
        let config = ChannelConfig::default();
        assert_eq!(config.command_for("manager").program, "claude-agent-acp");
        assert_eq!(config.command_for("reviewer").program, "claude-agent-acp");
    }

    #[test]
    fn a_role_can_be_pinned_to_another_binary() {
        let mut config = ChannelConfig::default();
        config
            .commands
            .insert("backend".into(), AgentCommand::new("codex-acp"));
        assert_eq!(config.command_for("backend").program, "codex-acp");
        assert_eq!(config.command_for("frontend").program, "claude-agent-acp");
    }

    #[test]
    fn a_command_renders_as_it_would_be_invoked() {
        let cmd = AgentCommand::new("npx").args(["-y", "@zed-industries/claude-agent-acp"]);
        assert_eq!(cmd.to_string(), "npx -y @zed-industries/claude-agent-acp");
        let (program, args, env) = cmd.as_parts();
        assert_eq!(program, "npx");
        assert_eq!(args, ["-y", "@zed-industries/claude-agent-acp"]);
        assert!(env.is_empty());
    }
}
