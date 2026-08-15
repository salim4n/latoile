//! Port allocation. First free port from a base upward; "free" means both
//! not handed out by this supervisor and actually bindable right now, so a
//! port taken by something outside LaToile is skipped too.

use crate::error::PreviewError;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;

/// Where previews start; the spec's own example serves on 4100.
pub const DEFAULT_BASE_PORT: u16 = 4100;
/// How far past the base we look before giving up — far more previews than
/// one user ever runs at once.
const RANGE: u16 = 100;

#[derive(Debug, Default)]
pub struct PortAllocator {
    in_use: HashSet<u16>,
}

impl PortAllocator {
    /// The first bindable port at or above `base`, registered as taken.
    /// Used when a refreshed preview keeps its port so the UI's URL survives.
    pub async fn take_except(
        &mut self,
        base: u16,
        preferred: Option<u16>,
    ) -> Result<u16, PreviewError> {
        if let Some(port) = preferred {
            if !self.in_use.contains(&port) && bindable(port).await {
                self.in_use.insert(port);
                return Ok(port);
            }
        }
        for offset in 0..RANGE {
            let port = base.saturating_add(offset);
            if self.in_use.contains(&port) || !bindable(port).await {
                continue;
            }
            self.in_use.insert(port);
            return Ok(port);
        }
        Err(PreviewError::NoFreePort)
    }

    /// Hand a port back; dead previews free their slot.
    pub fn release(&mut self, port: u16) {
        self.in_use.remove(&port);
    }

    #[cfg(test)]
    fn is_taken(&self, port: u16) -> bool {
        self.in_use.contains(&port)
    }
}

/// A port is bindable if we can listen on it and let go. There is an
/// inherent race between this probe and the dev server binding — accepted:
/// the readiness probe catches a loser, and the alternative (passing a bound
/// socket into the child) breaks on most dev servers.
async fn bindable(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed, non-overlapping windows below the OS ephemeral range: the
    /// allocator scans `base..base+99`, so tests stay 100 apart and never
    /// probe each other's ports — parallel-safe without OS-assigned ports
    /// (those can land anywhere and race another test's window).
    async fn hold_port(base: u16) -> (TcpListener, u16) {
        for port in base..base + 50 {
            if let Ok(listener) =
                TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
            {
                return (listener, port);
            }
        }
        panic!("no test port free above {base}");
    }

    #[tokio::test]
    async fn ports_are_handed_out_in_order_without_repeats() {
        let mut alloc = PortAllocator::default();
        let a = alloc.take_except(35100, None).await.unwrap();
        let b = alloc.take_except(35100, None).await.unwrap();
        assert!(a >= 35100 && b > a && b < 35200, "{a} then {b}");
    }

    #[tokio::test]
    async fn a_port_bound_by_someone_else_is_skipped() {
        let (_held, taken) = hold_port(35300).await;

        let mut alloc = PortAllocator::default();
        let port = alloc.take_except(taken, None).await.unwrap();
        assert_ne!(port, taken, "an occupied port must not be handed out");
        assert!(port > taken && port < 35400);
    }

    #[tokio::test]
    async fn a_released_port_is_handed_out_again() {
        let mut alloc = PortAllocator::default();
        let a = alloc.take_except(35200, None).await.unwrap();
        alloc.release(a);
        assert!(!alloc.is_taken(a));
        // The port space is shared with the whole machine: if a transient
        // outsider grabbed it in this exact instant, skipping it was correct
        // too — so retry rather than demand it back this millisecond.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let b = alloc.take_except(35200, None).await.unwrap();
            if b == a {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{a} never came back");
            alloc.release(b);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn the_preferred_port_wins_when_free() {
        let mut alloc = PortAllocator::default();
        let port = alloc.take_except(35400, Some(35405)).await.unwrap();
        assert_eq!(port, 35405);
    }

    #[tokio::test]
    async fn the_preferred_port_loses_when_taken() {
        let (_held, taken) = hold_port(35500).await;

        let mut alloc = PortAllocator::default();
        let port = alloc.take_except(35500, Some(taken)).await.unwrap();
        assert_ne!(port, taken);
        assert!((35500..35600).contains(&port));
    }
}
