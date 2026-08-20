//! In-memory rendezvous between an ACP request handler and the owner's HTTP
//! decision. SQLite owns the audit record; this broker owns only live
//! responders, which intentionally disappear on server restart.

use latoile_core::ids::RunId;
use latoile_core::ports::PermissionRequest;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Default)]
pub struct PermissionBroker {
    inner: Arc<Mutex<Registry>>,
}

#[derive(Default)]
struct Registry {
    pending: HashMap<String, Pending>,
    expired: HashMap<String, Expired>,
}

struct Pending {
    run_id: RunId,
    request: PermissionRequest,
    decision: oneshot::Sender<bool>,
}

struct Expired {
    run_id: RunId,
    request: PermissionRequest,
}

impl PermissionBroker {
    pub fn register(
        &self,
        run_id: RunId,
        summary: String,
    ) -> (PermissionRequest, oneshot::Receiver<bool>) {
        let request = PermissionRequest {
            id: format!("perm-{}", ulid::Ulid::new()),
            summary,
        };
        let (decision, receiver) = oneshot::channel();
        self.inner
            .lock()
            .expect("permission broker poisoned")
            .pending
            .insert(
                request.id.clone(),
                Pending {
                    run_id,
                    request: request.clone(),
                    decision,
                },
            );
        (request, receiver)
    }

    pub fn pending_for_run(&self, run_id: &RunId) -> Option<PermissionRequest> {
        self.inner
            .lock()
            .expect("permission broker poisoned")
            .pending
            .values()
            .find(|pending| &pending.run_id == run_id)
            .map(|pending| pending.request.clone())
    }

    pub fn expired_for_run(&self, run_id: &RunId) -> Option<PermissionRequest> {
        self.inner
            .lock()
            .expect("permission broker poisoned")
            .expired
            .values()
            .find(|expired| &expired.run_id == run_id)
            .map(|expired| expired.request.clone())
    }

    pub fn resolve(&self, run_id: &RunId, request_id: &str, granted: bool) -> Result<(), String> {
        let pending = {
            let mut registry = self.inner.lock().expect("permission broker poisoned");
            let belongs = registry
                .pending
                .get(request_id)
                .is_some_and(|pending| &pending.run_id == run_id);
            if !belongs {
                return Err("pending ACP permission request was not found".into());
            }
            registry.pending.remove(request_id).expect("checked above")
        };
        pending
            .decision
            .send(granted)
            .map_err(|_| "pending ACP permission session is no longer alive".into())
    }

    pub fn expire(&self, request_id: &str) {
        let mut registry = self.inner.lock().expect("permission broker poisoned");
        if let Some(pending) = registry.pending.remove(request_id) {
            registry.expired.insert(
                request_id.to_string(),
                Expired {
                    run_id: pending.run_id,
                    request: pending.request,
                },
            );
        }
    }

    pub fn acknowledge_expiry(&self, run_id: &RunId, request_id: &str) {
        let mut registry = self.inner.lock().expect("permission broker poisoned");
        if registry
            .expired
            .get(request_id)
            .is_some_and(|expired| &expired.run_id == run_id)
        {
            registry.expired.remove(request_id);
        }
    }

    /// Reject and forget every live responder for a run. Sending before the
    /// process is aborted lets a cooperative agent finish the request; the
    /// process-group guard remains the final safety net.
    pub fn cancel_run(&self, run_id: &RunId) {
        self.finish_run(run_id);
        self.inner
            .lock()
            .expect("permission broker poisoned")
            .expired
            .retain(|_, expired| &expired.run_id != run_id);
    }

    /// A terminal prompt has no live responder left. Preserve an expiry long
    /// enough for the driver to journal it, but remove all pending senders.
    pub fn finish_run(&self, run_id: &RunId) {
        let mut registry = self.inner.lock().expect("permission broker poisoned");
        let ids = registry
            .pending
            .iter()
            .filter(|(_, pending)| &pending.run_id == run_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(pending) = registry.pending.remove(&id) {
                let run_id = pending.run_id;
                let request = pending.request;
                let _ = pending.decision.send(false);
                registry.expired.insert(id, Expired { run_id, request });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunId {
        RunId::new("run-1").unwrap()
    }

    #[tokio::test]
    async fn a_pending_request_is_resolved_exactly_once() {
        let broker = PermissionBroker::default();
        let (request, receiver) = broker.register(run(), "Modify project files".into());
        assert_eq!(broker.pending_for_run(&run()), Some(request.clone()));

        broker.resolve(&run(), &request.id, true).unwrap();
        assert!(receiver.await.unwrap());
        assert!(broker.resolve(&run(), &request.id, true).is_err());
        assert!(broker.pending_for_run(&run()).is_none());
    }

    #[tokio::test]
    async fn timeout_and_cancel_leave_no_live_responder() {
        let broker = PermissionBroker::default();
        let (expired, receiver) = broker.register(run(), "Execute a command".into());
        broker.expire(&expired.id);
        assert!(receiver.await.is_err());
        assert_eq!(broker.expired_for_run(&run()), Some(expired.clone()));
        broker.acknowledge_expiry(&run(), &expired.id);
        assert!(broker.expired_for_run(&run()).is_none());

        let (cancelled, receiver) = broker.register(run(), "Modify project files".into());
        broker.cancel_run(&run());
        assert!(!receiver.await.unwrap());
        assert!(broker.resolve(&run(), &cancelled.id, true).is_err());
    }
}
