//! A bounded ring of log lines. The dev server's stdout/stderr stream here;
//! the server crate will later expose a snapshot over SSE. Bounded, because a
//! vite server left running for a week must not become a memory leak.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// How many lines a preview keeps by default.
pub const DEFAULT_CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
pub struct LogRing {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            capacity: capacity.max(1),
        }
    }

    /// Append one line; the oldest line falls off when full. Cheap to call
    /// from a reader task: the lock is held for a push and maybe a pop.
    pub fn push(&self, line: String) {
        let mut lines = self.inner.lock().expect("log ring poisoned");
        if lines.len() == self.capacity {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// Oldest first, as it should render.
    pub fn snapshot(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("log ring poisoned")
            .iter()
            .cloned()
            .collect()
    }

    /// The last `n` lines joined — for error messages, where the tail is the
    /// useful part.
    pub fn tail(&self, n: usize) -> String {
        let lines = self.inner.lock().expect("log ring poisoned");
        lines
            .iter()
            .rev()
            .take(n)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_come_back_oldest_first() {
        let ring = LogRing::new(10);
        ring.push("un".into());
        ring.push("deux".into());
        assert_eq!(ring.snapshot(), ["un", "deux"]);
    }

    #[test]
    fn the_ring_stays_bounded() {
        let ring = LogRing::new(3);
        for i in 0..10 {
            ring.push(format!("line {i}"));
        }
        assert_eq!(ring.snapshot(), ["line 7", "line 8", "line 9"]);
    }

    #[test]
    fn the_tail_is_the_most_recent_lines() {
        let ring = LogRing::new(10);
        for i in 0..5 {
            ring.push(format!("l{i}"));
        }
        assert_eq!(ring.tail(2), "l3\nl4");
        assert_eq!(ring.tail(50), "l0\nl1\nl2\nl3\nl4");
    }

    #[test]
    fn clones_share_one_buffer() {
        let ring = LogRing::new(5);
        let writer = ring.clone();
        writer.push("hello".into());
        assert_eq!(ring.snapshot(), ["hello"]);
    }
}
