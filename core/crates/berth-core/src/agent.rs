//! Agent lifecycle: the state machine is the product, so it lives here and
//! not in the UI.
//!
//! Rules that matter:
//! - `unknown` is honest, not empty: a detected-but-unregistered agent.
//! - illegal transitions are rejected, never coerced.
//! - silence is a failure mode: active states that stop reporting become
//!   `blocked` after a TTL, with `reason: "stale"`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Unknown,
    Idle,
    Thinking,
    Tool,
    WaitingInput,
    Blocked,
    Done,
    Errored,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Tool => "tool",
            Self::WaitingInput => "waiting_input",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Errored => "errored",
        }
    }

    /// States that require a heartbeat, or they get escalated.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Thinking | Self::Tool)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Errored)
    }

    /// States the user must look at.
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::WaitingInput | Self::Blocked | Self::Errored)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    #[default]
    None,
    Info,
    NeedsInput,
    Error,
}

impl Attention {
    pub fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Info => 1,
            Self::NeedsInput => 2,
            Self::Error => 3,
        }
    }

    pub fn is_unread(self) -> bool {
        self.rank() >= 2
    }
}

/// Attention is derived from state; callers may not set it directly.
pub fn derive_attention(state: AgentState) -> Attention {
    match state {
        AgentState::WaitingInput => Attention::NeedsInput,
        AgentState::Blocked | AgentState::Errored => Attention::Error,
        AgentState::Done => Attention::Info,
        _ => Attention::None,
    }
}

/// Is `to` reachable from `from`? Identity is always legal (idempotent update).
pub fn transition_allowed(from: AgentState, to: AgentState) -> bool {
    use AgentState::*;
    if from == to {
        return true;
    }
    match from {
        // A fresh or recovered agent may claim any state.
        Unknown => true,
        Idle => matches!(
            to,
            Thinking | WaitingInput | Blocked | Done | Errored | Unknown
        ),
        Thinking => matches!(
            to,
            Tool | Idle | WaitingInput | Blocked | Done | Errored | Unknown
        ),
        Tool => matches!(
            to,
            Thinking | Idle | WaitingInput | Blocked | Done | Errored | Unknown
        ),
        WaitingInput => matches!(to, Thinking | Idle | Done | Blocked | Errored | Unknown),
        Blocked => matches!(
            to,
            Thinking | Idle | WaitingInput | Done | Errored | Unknown
        ),
        // Terminal states can be resumed into a new turn.
        Done | Errored => matches!(to, Idle | Thinking | WaitingInput | Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_happy_path() {
        let path = [
            AgentState::Unknown,
            AgentState::Idle,
            AgentState::Thinking,
            AgentState::Tool,
            AgentState::Thinking,
            AgentState::WaitingInput,
            AgentState::Thinking,
            AgentState::Done,
        ];
        for pair in path.windows(2) {
            assert!(
                transition_allowed(pair[0], pair[1]),
                "{} -> {} should be legal",
                pair[0].as_str(),
                pair[1].as_str()
            );
        }
    }

    #[test]
    fn blocked_cannot_go_back_to_tool_without_thinking() {
        assert!(!transition_allowed(AgentState::Blocked, AgentState::Tool));
    }

    #[test]
    fn terminal_states_cannot_go_straight_to_tool() {
        assert!(!transition_allowed(AgentState::Done, AgentState::Tool));
        assert!(!transition_allowed(AgentState::Errored, AgentState::Tool));
    }

    #[test]
    fn attention_derivation() {
        assert_eq!(
            derive_attention(AgentState::WaitingInput),
            Attention::NeedsInput
        );
        assert_eq!(derive_attention(AgentState::Errored), Attention::Error);
        assert_eq!(derive_attention(AgentState::Done), Attention::Info);
        assert_eq!(derive_attention(AgentState::Thinking), Attention::None);
    }
}
