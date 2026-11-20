//! Frame hold for synchronized output (DECSET 2026).
//!
//! A program brackets a frame with `CSI ? 2026 h` and `CSI ? 2026 l` so a
//! terminal can show the finished frame instead of a half-updated grid. The
//! window keeps the last painted frame for a surface while the program holds
//! it, and releases the hold after [`SYNC_OUTPUT_TIMEOUT`] so a client that
//! crashes before its reset cannot freeze the pane.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use berth_headless::ScreenView;

/// How long a pane may keep its previous frame after a program asks for a
/// hold. A healthy client ends a frame in milliseconds.
pub const SYNC_OUTPUT_TIMEOUT: Duration = Duration::from_millis(500);

/// The painted frame and the hold state of every surface.
#[derive(Default)]
pub struct FrameHold {
    holds: HashMap<String, Hold>,
    frames: HashMap<String, Arc<ScreenView>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Hold {
    #[default]
    Released,
    /// The program set 2026; the pane shows the frame from `since`.
    Held { since: Instant },
    /// The hold outlived the timeout. Frames paint again until the program
    /// clears 2026 and starts a new hold.
    Expired,
}

impl FrameHold {
    /// Record a fresh screen and return the frame to paint.
    pub fn frame_to_paint(
        &mut self,
        surface_id: &str,
        fresh: ScreenView,
        now: Instant,
    ) -> Arc<ScreenView> {
        let is_held = {
            let hold = self.holds.entry(surface_id.to_owned()).or_default();
            if fresh.is_synchronized_output_enabled {
                hold.begin(now);
            } else {
                hold.release();
            }
            hold.is_held(now)
        };
        if is_held && let Some(cached) = self.frames.get(surface_id) {
            return Arc::clone(cached);
        }
        let fresh = Arc::new(fresh);
        self.frames
            .insert(surface_id.to_owned(), Arc::clone(&fresh));
        fresh
    }

    /// Release holds past the timeout. Returns whether one was released.
    pub fn release_expired(&mut self, now: Instant) -> bool {
        let mut released = false;
        for hold in self.holds.values_mut() {
            released |= hold.release_if_expired(now);
        }
        released
    }

    /// When the earliest active hold times out.
    pub fn deadline(&self) -> Option<Instant> {
        self.holds.values().filter_map(Hold::deadline).min()
    }

    /// Forget a closed surface.
    pub fn remove(&mut self, surface_id: &str) {
        self.holds.remove(surface_id);
        self.frames.remove(surface_id);
    }
}

impl Hold {
    fn begin(&mut self, now: Instant) {
        if matches!(self, Hold::Released) {
            *self = Hold::Held { since: now };
        }
    }

    fn release(&mut self) {
        *self = Hold::Released;
    }

    fn is_held(&self, now: Instant) -> bool {
        match self {
            Hold::Held { since } => now.saturating_duration_since(*since) < SYNC_OUTPUT_TIMEOUT,
            Hold::Released | Hold::Expired => false,
        }
    }

    fn release_if_expired(&mut self, now: Instant) -> bool {
        if matches!(self, Hold::Held { .. }) && !self.is_held(now) {
            *self = Hold::Expired;
            return true;
        }
        false
    }

    fn deadline(&self) -> Option<Instant> {
        match self {
            Hold::Held { since } => Some(*since + SYNC_OUTPUT_TIMEOUT),
            Hold::Released | Hold::Expired => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use berth_headless::CellView;

    use super::*;

    fn view(marker: &str) -> ScreenView {
        ScreenView {
            cols: 1,
            rows: 1,
            cells: vec![CellView {
                contents: marker.to_string(),
                ..CellView::default()
            }],
            ..ScreenView::default()
        }
    }

    /// A screen the program is still drawing under DECSET 2026.
    fn held(marker: &str) -> ScreenView {
        ScreenView {
            is_synchronized_output_enabled: true,
            ..view(marker)
        }
    }

    fn painted(frame: &ScreenView) -> &str {
        &frame.cells[0].contents
    }

    #[test]
    fn given_the_first_frame_of_a_hold_then_it_paints_and_is_kept() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        let first = hold.frame_to_paint("s", held("first"), now);
        let second = hold.frame_to_paint("s", held("second"), now + Duration::from_millis(10));
        assert_eq!(painted(&first), "first");
        assert_eq!(painted(&second), "first");
    }

    #[test]
    fn given_synchronized_output_ends_then_the_next_frame_paints() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("s", held("first"), now);
        let released = hold.frame_to_paint("s", view("second"), now + Duration::from_millis(10));
        assert_eq!(painted(&released), "second");
    }

    #[test]
    fn given_a_hold_past_the_timeout_then_the_next_frame_paints() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("s", held("first"), now);
        let late = now + SYNC_OUTPUT_TIMEOUT;
        assert!(hold.release_expired(late), "the expired hold is reported");
        let released = hold.frame_to_paint("s", held("second"), late);
        assert_eq!(painted(&released), "second");
        assert_eq!(hold.deadline(), None);
    }

    #[test]
    fn given_an_expired_hold_still_set_then_it_does_not_rearm() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("s", held("first"), now);
        let late = now + SYNC_OUTPUT_TIMEOUT;
        hold.release_expired(late);
        let second = hold.frame_to_paint("s", held("second"), late);
        let third = hold.frame_to_paint("s", held("third"), late + Duration::from_millis(10));
        assert_eq!(painted(&second), "second");
        assert_eq!(painted(&third), "third");
    }

    #[test]
    fn given_a_new_hold_after_the_last_one_cleared_then_it_holds_the_previous_frame() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("s", held("first"), now);
        let second = hold.frame_to_paint("s", view("second"), now + Duration::from_millis(5));
        let third = hold.frame_to_paint("s", held("third"), now + Duration::from_millis(10));
        let fourth = hold.frame_to_paint("s", held("fourth"), now + Duration::from_millis(15));
        assert_eq!(painted(&second), "second");
        assert_eq!(painted(&third), "second");
        assert_eq!(painted(&fourth), "second");
    }

    #[test]
    fn given_two_surfaces_then_the_earliest_hold_is_the_deadline() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("a", held("a"), now + Duration::from_millis(30));
        hold.frame_to_paint("b", held("b"), now);
        assert_eq!(hold.deadline(), Some(now + SYNC_OUTPUT_TIMEOUT));
    }

    #[test]
    fn given_a_removed_surface_then_its_frame_and_hold_are_forgotten() {
        let now = Instant::now();
        let mut hold = FrameHold::default();
        hold.frame_to_paint("s", held("first"), now);
        hold.remove("s");
        assert_eq!(hold.deadline(), None);
        let second = hold.frame_to_paint("s", held("second"), now);
        assert_eq!(painted(&second), "second");
    }
}
