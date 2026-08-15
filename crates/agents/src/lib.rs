//! The agent channel. Every agent process in the system is spawned here:
//! supervised (kill on drop, process group, registry) and spoken to over the
//! Agent Client Protocol.
//!
//! Two lifecycles live here: the persistent per-project Manager session
//! (resumed on each message) and ephemeral executor runs (spawn → task →
//! exit). Permissions follow allow/approval/reject heuristics — auto-reject
//! on `.env`, absolute paths, `docker` — with anything non-trivial routed to
//! the human approval queue.
//!
//! Implements ports defined in `latoile-core`. ACP version mismatches are
//! refused at handshake (pinned version + canary prompt).
