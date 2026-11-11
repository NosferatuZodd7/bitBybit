//! One mutating operation per category at a time; waiters give up after a timeout (→ Busy).

use crate::operation::Category;
use std::collections::HashSet;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ConcurrencyGate {
    busy: Mutex<HashSet<&'static str>>,
    released: Condvar,
}

pub struct Lease<'a> {
    gate: &'a ConcurrencyGate,
    name: Option<&'static str>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(name) = self.name {
            if let Ok(mut busy) = self.gate.busy.lock() {
                busy.remove(name);
            }
            self.gate.released.notify_all();
        }
    }
}

impl ConcurrencyGate {
    /// `None` when the category stayed busy for `wait`.
    pub fn try_enter(&self, category: &Category, wait: Duration) -> Option<Lease<'_>> {
        if category.is_none() {
            return Some(Lease { gate: self, name: None });
        }
        let name = category.name();
        let deadline = Instant::now() + wait;
        let mut busy = self.busy.lock().ok()?;
        while busy.contains(name) {
            let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
            busy = self.released.wait_timeout(busy, left).ok()?.0;
        }
        busy.insert(name);
        Some(Lease { gate: self, name: Some(name) })
    }

    pub fn is_busy(&self, category: &Category) -> bool {
        !category.is_none() && self.busy.lock().is_ok_and(|b| b.contains(category.name()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_per_category_and_independent_categories() {
        let gate = ConcurrencyGate::default();
        let lease = gate.try_enter(&Category::NETWORK, Duration::ZERO).unwrap();
        assert!(gate.is_busy(&Category::NETWORK));
        assert!(gate.try_enter(&Category::NETWORK, Duration::from_millis(20)).is_none());
        assert!(gate.try_enter(&Category::AUDIO, Duration::ZERO).is_some());
        assert!(gate.try_enter(&Category::NONE, Duration::ZERO).is_some());
        std::thread::scope(|s| {
            let waiter = s.spawn(|| gate.try_enter(&Category::NETWORK, Duration::from_secs(5)).is_some());
            std::thread::sleep(Duration::from_millis(30));
            drop(lease);
            assert!(waiter.join().unwrap(), "released lease wakes the waiter");
        });
        assert!(!gate.is_busy(&Category::NETWORK));
    }
}
