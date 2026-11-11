//! Broker → client notifications through long polling (`broker.events.poll`). Bounded ring: a slow client sees
//! `truncated` instead of the broker holding unbounded memory.

use bitbybit_core::{Error, LocalizedText, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerEvent {
    pub sequence: u64,
    /// Same syntax as operation names.
    pub topic: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<LocalizedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventsPage {
    pub events: Vec<BrokerEvent>,
    pub latest_sequence: u64,
    /// Events after the requested sequence were dropped from the ring.
    pub truncated: bool,
}

struct Ring {
    events: VecDeque<BrokerEvent>,
    sequence: u64,
}

pub struct EventHub {
    ring: Mutex<Ring>,
    published: Condvar,
    capacity: usize,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            ring: Mutex::new(Ring { events: VecDeque::new(), sequence: 0 }),
            published: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn latest_sequence(&self) -> u64 {
        self.ring.lock().map_or(0, |r| r.sequence)
    }

    pub fn publish(&self, topic: &str, message: Option<LocalizedText>, data: Option<Value>) -> Result<()> {
        if !crate::envelope::is_valid_operation_name(topic) {
            return Err(Error::invalid(format!("Event topic '{topic}' is not valid.")));
        }
        let mut ring = self.ring.lock().map_err(|_| Error::invalid("event hub poisoned"))?;
        ring.sequence += 1;
        let event = BrokerEvent {
            sequence: ring.sequence,
            topic: topic.to_owned(),
            at: OffsetDateTime::now_utc(),
            message,
            data,
        };
        ring.events.push_back(event);
        while ring.events.len() > self.capacity {
            ring.events.pop_front();
        }
        drop(ring);
        self.published.notify_all();
        Ok(())
    }

    fn collect(ring: &Ring, after: u64, max: usize) -> EventsPage {
        let events: Vec<BrokerEvent> = ring.events.iter().filter(|e| e.sequence > after).take(max).cloned().collect();
        let oldest = ring.events.front().map_or(ring.sequence + 1, |e| e.sequence);
        EventsPage { events, latest_sequence: ring.sequence, truncated: after + 1 < oldest && after < ring.sequence }
    }

    /// Waits up to `max_wait` for events after `after`.
    pub fn poll(&self, after: u64, max_events: usize, max_wait: Duration) -> EventsPage {
        let max = max_events.clamp(1, 500);
        let deadline = Instant::now() + max_wait;
        let Ok(mut ring) = self.ring.lock() else {
            return EventsPage { events: vec![], latest_sequence: 0, truncated: false };
        };
        loop {
            let page = Self::collect(&ring, after, max);
            let left = deadline.saturating_duration_since(Instant::now());
            if !page.events.is_empty() || left.is_zero() {
                return page;
            }
            match self.published.wait_timeout(ring, left) {
                Ok((r, _)) => ring = r,
                Err(_) => return page,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_new_events_waits_and_reports_truncation() {
        let hub = EventHub::new(3);
        assert!(hub.publish("Bad Topic", None, None).is_err());
        let empty = hub.poll(0, 10, Duration::from_millis(10));
        assert!(empty.events.is_empty() && !empty.truncated);
        std::thread::scope(|s| {
            let waiter = s.spawn(|| hub.poll(0, 10, Duration::from_secs(5)));
            std::thread::sleep(Duration::from_millis(30));
            hub.publish("network.sharing.changed", None, Some(serde_json::json!({"state": "on"}))).unwrap();
            let page = waiter.join().unwrap();
            assert_eq!(page.events.len(), 1);
            assert_eq!(page.events[0].topic, "network.sharing.changed");
        });
        for _ in 0..4 {
            hub.publish("audio.default.changed", None, None).unwrap();
        }
        let page = hub.poll(0, 10, Duration::ZERO);
        assert_eq!((page.events.len(), page.latest_sequence, page.truncated), (3, 5, true));
        let caught_up = hub.poll(5, 10, Duration::ZERO);
        assert!(caught_up.events.is_empty() && !caught_up.truncated);
        assert_eq!(hub.poll(3, 1, Duration::ZERO).events[0].sequence, 4);
    }
}
