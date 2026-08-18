//! Ports — the outgoing interfaces the domain and use cases need, implemented
//! by the adapter crates (`agents`, `preview`, `github`, `vault`, and the
//! persistence module in `app`).
//!
//! `async fn` in traits is native (Rust 1.75+): a future is not a runtime, so
//! `core` stays dependency-free while adapters stay async. No `async-trait`
//! crate, no async runtime. The `async_fn_in_trait` lint is allowed at the module
//! declaration (see lib.rs): LaToile is a single binary and every adapter is
//! `Send` by construction — if that ever changes, switch to `trait_variant`.
//!
//! Methods take and return domain types or plain data. Nothing here knows
//! about HTTP, SQL, or ACP frames.

use crate::approval::Approval;
use crate::conversation::{Conversation, Message};
use crate::delivery::Delivery;
use crate::event::NewEvent;
use crate::ids::{ApprovalId, ProjectId, RunId, TaskId};
use crate::preview::Preview;
use crate::project::Project;
use crate::run::Run;
use crate::spec::SpecVersion;
use crate::task::Task;

/// What adapters report when they fail. Deliberately opaque: a message for
/// logs, mapped to `{code, message}` at the HTTP edge — internal chains never
/// reach clients (architecture contract §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortError(pub String);

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PortError {}

pub type PortResult<T> = Result<T, PortError>;

// --- Persistence -----------------------------------------------------------

/// Each store trait covers one aggregate. `save` is an upsert; the store
/// enforces the partial-unique invariants (one active run per task, one
/// active preview per project, one approved spec per project) at the SQL
/// level, mirroring the domain guards.
pub trait ProjectStore {
    async fn get(&self, id: &ProjectId) -> PortResult<Option<Project>>;
    async fn list(&self) -> PortResult<Vec<Project>>;
    async fn save(&self, project: &Project) -> PortResult<()>;
}

pub trait TaskStore {
    async fn get(&self, id: &TaskId) -> PortResult<Option<Task>>;
    async fn list_for_project(&self, project: &ProjectId) -> PortResult<Vec<Task>>;
    async fn save(&self, task: &Task) -> PortResult<()>;
}

pub trait RunStore {
    async fn get(&self, id: &RunId) -> PortResult<Option<Run>>;
    async fn list_for_task(&self, task: &TaskId) -> PortResult<Vec<Run>>;
    /// The single active run on a task, if any (invariant §3.2.1).
    async fn active_for_task(&self, task: &TaskId) -> PortResult<Option<Run>>;
    async fn save(&self, run: &Run) -> PortResult<()>;
}

pub trait SpecStore {
    async fn approved_for_project(&self, project: &ProjectId) -> PortResult<Option<SpecVersion>>;
    async fn save(&self, spec: &SpecVersion) -> PortResult<()>;
}

pub trait ApprovalStore {
    async fn get(&self, id: &ApprovalId) -> PortResult<Option<Approval>>;
    async fn list_pending(&self) -> PortResult<Vec<Approval>>;
    async fn list_for_run(&self, run: &RunId) -> PortResult<Vec<Approval>>;
    async fn save(&self, approval: &Approval) -> PortResult<()>;
}

pub trait DeliveryStore {
    async fn get_for_project(&self, project: &ProjectId) -> PortResult<Option<Delivery>>;
    async fn save(&self, delivery: &Delivery) -> PortResult<()>;
}

pub trait PreviewStore {
    async fn active_for_project(&self, project: &ProjectId) -> PortResult<Option<Preview>>;
    async fn save(&self, preview: &Preview) -> PortResult<()>;
}

pub trait ConversationStore {
    async fn for_project(&self, project: &ProjectId) -> PortResult<Option<Conversation>>;
    async fn append(&self, message: &Message) -> PortResult<()>;
    async fn recent(&self, conversation: &ProjectId, limit: u32) -> PortResult<Vec<Message>>;
}

// --- Journal ---------------------------------------------------------------

/// The append-only event log. The store assigns `seq`; that cursor is what
/// the SSE stream resumes from.
pub trait EventLog {
    async fn append(&self, event: &NewEvent) -> PortResult<u64>;
    async fn since(&self, project: &ProjectId, after_seq: u64) -> PortResult<Vec<(u64, NewEvent)>>;
}

// --- Adapters --------------------------------------------------------------

/// What the Manager answered: the conversational reply plus the structured
/// actions it decided (JSON: tasks created, runs started…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerReply {
    pub content: String,
    pub actions: Option<String>,
}

/// A sanitized ACP permission request. Raw tool input deliberately stays in
/// the agent adapter; the application only receives an opaque request id and
/// an owner-readable operation class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: String,
    pub summary: String,
}

/// The agent channel. Two lifecycles: the persistent per-project Manager
/// session, and ephemeral executor runs.
pub trait AgentChannel {
    /// Resume the project's Manager session with a new user message.
    async fn tell_manager(&self, project: &ProjectId, message: &str) -> PortResult<ManagerReply>;
    /// Spawn an executor run in its project's checkout. The project is
    /// explicit because a new task/run is intentionally not persisted until
    /// the ACP handshake succeeds.
    async fn start_run(&self, project: &ProjectId, run: &Run, prompt: &str) -> PortResult<String>;
    /// Resolve the exact pending ACP permission request. Implementations must
    /// consume it at most once; a missing/lost request is an error.
    async fn resolve_permission(
        &self,
        _run: &RunId,
        _request_id: &str,
        _granted: bool,
    ) -> PortResult<()> {
        Err(PortError("permission resolution is not supported".into()))
    }
    async fn cancel_run(&self, run: &RunId) -> PortResult<()>;
}

/// Dev-server supervision for live previews.
pub trait PreviewSupervisor {
    /// Start (or recycle) the project's dev server; returns pid and port.
    async fn ensure(
        &self,
        preview: &Preview,
        dev_command: &str,
        working_dir: &str,
    ) -> PortResult<(u32, u16)>;
    async fn stop(&self, preview: &Preview) -> PortResult<()>;
}

/// A repository as shown in the project picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub full_name: String,
    pub description: Option<String>,
    pub private: bool,
}

pub trait GitHubClient {
    async fn list_repos(&self) -> PortResult<Vec<RepoInfo>>;
    async fn open_pull_request(&self, repo: &str, head: &str, base: &str) -> PortResult<String>;
    async fn find_open_pull_request(
        &self,
        repo: &str,
        head: &str,
        base: &str,
    ) -> PortResult<Option<String>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishWorkBranchInput {
    pub repo: String,
    pub checkout: String,
    pub work_branch: String,
    /// Executor commit SHAs whose Reviewer approvals authorized delivery.
    /// The publisher proves every one is an ancestor of the pushed HEAD.
    pub approved_shas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedWorkBranch {
    pub work_branch: String,
    pub local_sha: String,
    pub remote_sha: String,
}

/// Local Git verification and the one non-force branch push. API PR creation
/// remains a separate GitHub port so the application controls ordering.
pub trait WorkBranchPublisher {
    async fn verify_and_push(
        &self,
        input: &PublishWorkBranchInput,
    ) -> PortResult<PublishedWorkBranch>;
}

/// Input for provisioning the one V1 checkout attached to a project.
/// Filesystem layout and Git authentication stay inside the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionWorkspaceInput {
    pub repo: String,
    pub slug: String,
    pub work_branch: String,
    /// Optional owner override. When absent, the adapter detects the command
    /// from the checked-out repository and falls back to an actionable
    /// command that explains what must be configured.
    pub dev_command: Option<String>,
}

/// Canonical checkout facts discovered by the adapter and persisted on the
/// project. The browser never supplies host paths or branch truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedWorkspace {
    pub default_branch: String,
    pub work_branch: String,
    pub local_path: String,
    pub dev_command: String,
}

pub trait WorkspaceProvisioner {
    async fn provision(&self, input: &ProvisionWorkspaceInput) -> PortResult<ProvisionedWorkspace>;
}

/// Secret resolution. Values flow to adapters, never to logs or clients.
pub trait SecretStore {
    async fn get(&self, name: &str) -> PortResult<Option<String>>;
    async fn put(&self, name: &str, value: &str) -> PortResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-level proof the ports are implementable without any runtime:
    // a stub implementing two ports, never awaited.
    struct Stub;

    impl EventLog for Stub {
        async fn append(&self, _event: &NewEvent) -> PortResult<u64> {
            Ok(1)
        }
        async fn since(&self, _p: &ProjectId, _after: u64) -> PortResult<Vec<(u64, NewEvent)>> {
            Ok(vec![])
        }
    }

    impl SecretStore for Stub {
        async fn get(&self, _name: &str) -> PortResult<Option<String>> {
            Ok(None)
        }
        async fn put(&self, _name: &str, _value: &str) -> PortResult<()> {
            Ok(())
        }
    }

    #[test]
    fn ports_are_implementable() {
        fn assert_ports(_: &impl EventLog, _: &impl SecretStore) {}
        assert_ports(&Stub, &Stub);
    }
}
