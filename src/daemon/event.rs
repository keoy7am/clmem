use std::collections::VecDeque;

use crate::models::Event;

/// Simple publish/subscribe event bus with bounded history.
///
/// Stores recent events in a ring buffer for query by IPC clients
/// and future TUI subscribers.
pub struct EventBus {
    events: VecDeque<Event>,
    max_events: usize,
}

impl EventBus {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events),
            max_events,
        }
    }

    /// Publish a single event, evicting the oldest if at capacity.
    pub fn publish(&mut self, event: Event) {
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        tracing::debug!(kind = ?event.kind, "Event published");
        self.events.push_back(event);
    }

    /// Publish multiple events at once.
    pub fn publish_many(&mut self, events: Vec<Event>) {
        for event in events {
            self.publish(event);
        }
    }

    /// Return the most recent `n` events (or all if fewer exist).
    pub fn get_recent(&self, n: usize) -> Vec<Event> {
        let len = self.events.len();
        let start = len.saturating_sub(n);
        self.events.iter().skip(start).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EventKind;

    #[test]
    fn test_publish_and_retrieve() {
        let mut bus = EventBus::new(10);
        bus.publish(Event::new(EventKind::DaemonStarted));
        bus.publish(Event::new(EventKind::DaemonStopped));

        let recent = bus.get_recent(5);
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut bus = EventBus::new(3);
        for i in 0..5 {
            bus.publish(Event::new(EventKind::ProcessDiscovered {
                pid: i,
                name: format!("proc-{i}"),
            }));
        }
        let recent = bus.get_recent(10);
        assert_eq!(recent.len(), 3);
        // Should have the last 3 events (pids 2, 3, 4)
        if let EventKind::ProcessDiscovered { pid, .. } = &recent[0].kind {
            assert_eq!(*pid, 2);
        } else {
            panic!("Expected ProcessDiscovered");
        }
    }

    #[test]
    fn test_publish_many() {
        let mut bus = EventBus::new(10);
        let events = vec![
            Event::new(EventKind::DaemonStarted),
            Event::new(EventKind::DaemonStopped),
        ];
        bus.publish_many(events);
        assert_eq!(bus.get_recent(10).len(), 2);
    }
}
