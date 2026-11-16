//! Interruptible sleep shared between a worker thread and whoever wants to wake or stop it.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Default)]
struct State {
    refresh: bool,
    stopped: bool,
}

#[derive(Default)]
pub struct Wake {
    state: Mutex<State>,
    signal: Condvar,
}

impl Wake {
    fn lock(&self) -> MutexGuard<'_, State> {
        // The state is two flags; a panic elsewhere cannot leave it inconsistent.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ask the worker to do its work now instead of waiting for the next interval.
    pub fn notify(&self) {
        self.lock().refresh = true;
        self.signal.notify_all();
    }

    /// Ask the worker to exit.
    pub fn stop(&self) {
        self.lock().stopped = true;
        self.signal.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// Consume a pending refresh request without sleeping.
    pub fn take_refresh(&self) -> bool {
        std::mem::take(&mut self.lock().refresh)
    }

    /// Sleep up to `duration`. Returns `true` when woken early by [`notify`](Self::notify)
    /// or [`stop`](Self::stop).
    pub fn sleep(&self, duration: Duration) -> bool {
        let guard = self.lock();
        let (mut guard, _) = self
            .signal
            .wait_timeout_while(guard, duration, |s| !s.refresh && !s.stopped)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut guard.refresh) || guard.stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn notify_interrupts_sleep() {
        let wake = Arc::new(Wake::default());
        let other = wake.clone();
        let started = Instant::now();
        std::thread::spawn(move || other.notify());
        assert!(wake.sleep(Duration::from_secs(30)));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(
            !wake.sleep(Duration::from_millis(1)),
            "the request was consumed"
        );
    }

    #[test]
    fn stop_is_sticky() {
        let wake = Wake::default();
        wake.stop();
        assert!(wake.is_stopped());
        assert!(wake.sleep(Duration::from_secs(30)));
        assert!(wake.sleep(Duration::from_secs(30)));
    }
}
