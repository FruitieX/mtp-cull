use std::time::{Duration, Instant};

/// Cancellation still runs every chunk; only UI notification is throttled.
pub struct ProgressGate {
    last: Instant,
}
impl ProgressGate {
    pub fn new() -> Self {
        Self {
            last: Instant::now() - Duration::from_secs(1),
        }
    }
    pub fn ready(&mut self, done: u64, total: u64) -> bool {
        if done == 0 || done == total || self.last.elapsed() >= Duration::from_millis(40) {
            self.last = Instant::now();
            true
        } else {
            false
        }
    }
}
