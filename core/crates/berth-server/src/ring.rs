//! Bounded event replay for socket subscribers (docs/03-protocol.md §3).
//!
//! Core events carry a process-wide monotonic `seq`; the ring keeps the last N
//! lines so a reconnect can ask for `events.subscribe {"since": N}`. A ring
//! overflow is reported as an explicit gap, never as silent loss.

/// One recorded event line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireEvent {
    pub seq: u64,
    pub event_type: String,
    pub line: String,
}

/// Answer to a replay request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventReplay {
    /// Events the ring no longer has.
    pub dropped: u64,
    /// Event lines after `since`, in order.
    pub lines: Vec<String>,
    /// Highest sequence considered by this replay.
    pub up_to: u64,
}

/// Fixed-capacity event history.
#[derive(Debug)]
pub struct EventRing {
    capacity: usize,
    events: Vec<WireEvent>,
    latest: u64,
}

impl Default for EventRing {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl EventRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: Vec::new(),
            latest: 0,
        }
    }

    /// Record every event line from one drained batch. Lines without a numeric
    /// `seq` (for example a handshake) are ignored for replay but still reach
    /// live subscribers through the normal broadcast path.
    pub fn record(&mut self, ndjson: &str) -> Vec<WireEvent> {
        let parsed: Vec<WireEvent> = ndjson
            .lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).ok()?;
                let seq = value.get("seq")?.as_u64()?;
                let event_type = value.get("event")?.as_str()?.to_string();
                Some(WireEvent {
                    seq,
                    event_type,
                    line: line.to_string(),
                })
            })
            .collect();
        if parsed.is_empty() {
            return parsed;
        }
        for event in &parsed {
            self.latest = self.latest.max(event.seq);
            self.events.push(event.clone());
        }
        if self.events.len() > self.capacity {
            let excess = self.events.len() - self.capacity;
            self.events.drain(..excess);
        }
        parsed
    }

    pub fn latest_seq(&self) -> u64 {
        self.latest
    }

    /// Events after `since`, optionally filtered by type.
    pub fn replay(&self, since: u64, types: Option<&[String]>) -> EventReplay {
        let oldest = self.events.first().map(|event| event.seq);
        let dropped = match oldest {
            Some(oldest) if since.saturating_add(1) < oldest => oldest - since - 1,
            None if self.latest > since => self.latest - since,
            _ => 0,
        };
        let lines = self
            .events
            .iter()
            .filter(|event| event.seq > since)
            .filter(|event| types.is_none_or(|types| types.iter().any(|t| t == &event.event_type)))
            .map(|event| event.line.clone())
            .collect();
        EventReplay {
            dropped,
            lines,
            up_to: self.latest,
        }
    }

    /// A fresh subscriber without `since` starts at the current head.
    pub fn head(&self) -> u64 {
        self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: u64, event: &str) -> String {
        format!(r#"{{"event":"{event}","seq":{seq},"data":{{}}}}"#)
    }

    #[test]
    fn records_only_sequenced_events() {
        let mut ring = EventRing::new(4);
        let recorded = ring.record(&format!("{}\n{}", line(1, "a"), r#"{"event":"hello"}"#));
        assert_eq!(recorded.len(), 1);
        assert_eq!(ring.head(), 1);
    }

    #[test]
    fn replay_returns_events_after_the_cursor_and_filters_by_type() {
        let mut ring = EventRing::new(8);
        ring.record(&[line(1, "a"), line(2, "b"), line(3, "a")].join("\n"));
        let all = ring.replay(0, None);
        assert_eq!(all.lines.len(), 3);
        assert_eq!(all.up_to, 3);
        let only_a = ring.replay(1, Some(&["a".to_string()]));
        assert_eq!(only_a.lines, vec![line(3, "a")]);
        assert_eq!(only_a.dropped, 0);
    }

    #[test]
    fn overflow_is_reported_as_a_gap() {
        let mut ring = EventRing::new(2);
        ring.record(&[line(1, "a"), line(2, "a"), line(3, "a")].join("\n"));
        let replay = ring.replay(0, None);
        // seq 1 fell out of the ring: the client is told it missed one event.
        assert_eq!(replay.dropped, 1);
        assert_eq!(replay.lines.len(), 2);
    }

    #[test]
    fn an_empty_ring_reports_every_lost_event_as_a_gap() {
        let mut ring = EventRing::new(2);
        ring.record(&line(5, "a"));
        ring.events.clear();
        let replay = ring.replay(0, None);
        assert_eq!(replay.dropped, 5);
        assert!(replay.lines.is_empty());
    }
}
