//! Pure mapping from ACP wire types to what LaToile understands. No I/O —
//! everything here is exhaustively unit-tested.
//!
//! Two mappings:
//!
//! - [`classify`]: `SessionUpdate` → [`AgentUpdate`], the channel's internal
//!   vocabulary (text for the transcript, tool activity for progress,
//!   permission requests for the approval flow).
//! - [`outcome_of`] / [`outcome_event`] / [`update_event`]: how a turn maps
//!   onto the domain's `EventKind`s. The domain declares no per-chunk events,
//!   so chunks and tool activity map to `None` — they are content, not
//!   journal entries. A failed or cancelled turn has no dedicated kind
//!   either; both journal as `RunFinished` with the outcome in the payload,
//!   because inventing domain events here is worse than a payload marker.

use agent_client_protocol::schema::v1::{ContentBlock, SessionUpdate, StopReason};
use latoile_core::event::EventKind;

/// One thing the agent told us, in the channel's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdate {
    /// A piece of the agent's reply — appended to the transcript.
    TextChunk(String),
    /// The agent's reasoning, kept separate from the reply.
    ThoughtChunk(String),
    /// A tool call started (edit, command, search…).
    ToolCallStarted { title: String },
    /// A tool call reached a terminal state.
    ToolCallFinished { title: Option<String> },
    /// The agent asked permission for something. Journaled as
    /// `ApprovalRequested`; the policy in `policy.rs` answers the agent.
    PermissionRequested { summary: String },
    /// The agent published or changed its plan.
    PlanUpdated,
    /// Anything else the protocol sent (usage, modes, commands…).
    Ignored(&'static str),
}

/// The text inside a content chunk, if it is text.
fn chunk_text(chunk: &agent_client_protocol::schema::v1::ContentChunk) -> Option<String> {
    match &chunk.content {
        ContentBlock::Text(text) => Some(text.text.clone()),
        _ => None,
    }
}

/// `SessionUpdate` → [`AgentUpdate`]. Non-text content blocks (images,
/// resources) inside message chunks are ignored: no LaToile surface shows
/// them.
pub fn classify(update: &SessionUpdate) -> AgentUpdate {
    use agent_client_protocol::schema::v1::ToolCallStatus;
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => match chunk_text(chunk) {
            Some(text) => AgentUpdate::TextChunk(text),
            None => AgentUpdate::Ignored("non_text_message_chunk"),
        },
        SessionUpdate::AgentThoughtChunk(chunk) => match chunk_text(chunk) {
            Some(text) => AgentUpdate::ThoughtChunk(text),
            None => AgentUpdate::Ignored("non_text_thought_chunk"),
        },
        SessionUpdate::UserMessageChunk(_) => AgentUpdate::Ignored("user_message_chunk"),
        SessionUpdate::ToolCall(call) => AgentUpdate::ToolCallStarted {
            title: call.title.clone(),
        },
        SessionUpdate::ToolCallUpdate(update) => {
            match update.fields.status {
                Some(ToolCallStatus::Completed | ToolCallStatus::Failed) => {
                    AgentUpdate::ToolCallFinished {
                        title: update.fields.title.clone(),
                    }
                }
                _ => AgentUpdate::Ignored("tool_call_progress"),
            }
        }
        SessionUpdate::Plan(_) => AgentUpdate::PlanUpdated,
        SessionUpdate::AvailableCommandsUpdate(_) => AgentUpdate::Ignored("available_commands"),
        SessionUpdate::CurrentModeUpdate(_) => AgentUpdate::Ignored("current_mode"),
        SessionUpdate::ConfigOptionUpdate(_) => AgentUpdate::Ignored("config_option"),
        SessionUpdate::SessionInfoUpdate(_) => AgentUpdate::Ignored("session_info"),
        SessionUpdate::UsageUpdate(_) => AgentUpdate::Ignored("usage"),
        _ => AgentUpdate::Ignored("unknown"),
    }
}

/// How a prompt turn ended, in run terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// `stop_reason: end_turn` — the agent finished its work.
    Finished,
    /// The client cancelled (`session/cancel`) and the agent confirmed.
    Cancelled,
    /// Token limit, turn limit, or refusal — the turn ended without the work
    /// being done.
    Failed,
}

pub fn outcome_of(stop: &StopReason) -> RunOutcome {
    match stop {
        StopReason::EndTurn => RunOutcome::Finished,
        StopReason::Cancelled => RunOutcome::Cancelled,
        StopReason::MaxTokens | StopReason::MaxTurnRequests | StopReason::Refusal => {
            RunOutcome::Failed
        }
        _ => RunOutcome::Failed,
    }
}

/// A finished run journals as `run_finished`; cancelled and failed runs have
/// no dedicated kind, so they journal as `run_finished` too, with the
/// outcome named in the payload (`{"outcome":"cancelled"|"error"}`). The
/// event log stays honest without growing the domain for the adapter's sake.
pub fn outcome_event(_outcome: RunOutcome) -> EventKind {
    EventKind::RunFinished
}

/// The one mid-turn signal with a domain counterpart: a permission request
/// is an approval the owner should hear about.
pub fn update_event(update: &AgentUpdate) -> Option<EventKind> {
    match update {
        AgentUpdate::PermissionRequested { .. } => Some(EventKind::ApprovalRequested),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        AvailableCommandsUpdate, ContentChunk, CurrentModeUpdate, Plan, SessionModeId,
        TextContent, ToolCall, ToolCallUpdate, ToolCallUpdateFields, UsageUpdate,
    };

    fn text_chunk(text: &str) -> ContentChunk {
        ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
    }

    #[test]
    fn message_chunks_become_text_and_thoughts_stay_separate() {
        assert_eq!(
            classify(&SessionUpdate::AgentMessageChunk(text_chunk("Bonjour"))),
            AgentUpdate::TextChunk("Bonjour".into())
        );
        assert_eq!(
            classify(&SessionUpdate::AgentThoughtChunk(text_chunk("réfléchis"))),
            AgentUpdate::ThoughtChunk("réfléchis".into())
        );
    }

    #[test]
    fn echoed_user_chunks_are_not_reply_text() {
        assert_eq!(
            classify(&SessionUpdate::UserMessageChunk(text_chunk("allo"))),
            AgentUpdate::Ignored("user_message_chunk")
        );
    }

    #[test]
    fn tool_calls_map_to_start_and_finish() {
        let started = SessionUpdate::ToolCall(ToolCall::new("tc1", "Edit src/main.rs"));
        assert_eq!(
            classify(&started),
            AgentUpdate::ToolCallStarted {
                title: "Edit src/main.rs".into()
            }
        );

        let mut finished = ToolCallUpdate::new("tc1", ToolCallUpdateFields::default());
        finished.fields.status = Some(agent_client_protocol::schema::v1::ToolCallStatus::Completed);
        assert!(matches!(
            classify(&SessionUpdate::ToolCallUpdate(finished)),
            AgentUpdate::ToolCallFinished { .. }
        ));

        let in_progress = ToolCallUpdate::new("tc2", ToolCallUpdateFields::default());
        assert_eq!(
            classify(&SessionUpdate::ToolCallUpdate(in_progress)),
            AgentUpdate::Ignored("tool_call_progress")
        );
    }

    #[test]
    fn plans_and_the_long_tail_are_classified_not_dropped() {
        assert_eq!(
            classify(&SessionUpdate::Plan(Plan::new(vec![]))),
            AgentUpdate::PlanUpdated
        );
        assert!(matches!(
            classify(&SessionUpdate::AvailableCommandsUpdate(
                AvailableCommandsUpdate::new(vec![])
            )),
            AgentUpdate::Ignored(_)
        ));
        assert!(matches!(
            classify(&SessionUpdate::UsageUpdate(UsageUpdate::new(0, 0))),
            AgentUpdate::Ignored(_)
        ));
        assert!(matches!(
            classify(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(SessionModeId::from("default")))),
            AgentUpdate::Ignored(_)
        ));
    }

    #[test]
    fn stop_reasons_become_run_outcomes() {
        assert_eq!(outcome_of(&StopReason::EndTurn), RunOutcome::Finished);
        assert_eq!(outcome_of(&StopReason::Cancelled), RunOutcome::Cancelled);
        assert_eq!(outcome_of(&StopReason::MaxTokens), RunOutcome::Failed);
        assert_eq!(outcome_of(&StopReason::MaxTurnRequests), RunOutcome::Failed);
        assert_eq!(outcome_of(&StopReason::Refusal), RunOutcome::Failed);
    }

    #[test]
    fn outcomes_journal_as_run_finished_with_no_invented_kinds() {
        for outcome in [RunOutcome::Finished, RunOutcome::Cancelled, RunOutcome::Failed] {
            assert_eq!(outcome_event(outcome), EventKind::RunFinished);
        }
    }

    #[test]
    fn only_permission_requests_have_a_domain_event() {
        let permission = AgentUpdate::PermissionRequested {
            summary: "docker compose up".into(),
        };
        assert_eq!(update_event(&permission), Some(EventKind::ApprovalRequested));
        assert_eq!(update_event(&AgentUpdate::TextChunk("x".into())), None);
        assert_eq!(update_event(&AgentUpdate::PlanUpdated), None);
    }
}
