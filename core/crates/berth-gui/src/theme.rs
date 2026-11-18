//! Visual tokens for the portable window.
//!
//! The values mirror the macOS shell's `Theme.swift`: one dark palette, the
//! system accent for focus, and the attention colors whose meaning the core
//! already defines. Views read these instead of hard-coding colors.

/// Packed 0x00RRGGBB.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub terminal_bg: u32,
    pub terminal_fg: u32,
    pub pane_border: u32,
    pub pane_border_focused: u32,
    pub tab_bar_bg: u32,
    pub tab_active_bg: u32,
    pub tab_text: u32,
    pub tab_text_dim: u32,
    pub sidebar_bg: u32,
    pub sidebar_active: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_tertiary: u32,
    pub attention_error: u32,
    pub attention_input: u32,
    pub success: u32,
    pub accent: u32,
    pub selection: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            terminal_bg: 0x0000_0000,
            terminal_fg: 0x00e6_e6ee,
            pane_border: 0x0024_2430,
            pane_border_focused: 0x000a_84ff,
            tab_bar_bg: 0x0016_161c,
            tab_active_bg: 0x002a_2a34,
            tab_text: 0x00c8_c8d8,
            tab_text_dim: 0x0078_7888,
            sidebar_bg: 0x0017_171e,
            sidebar_active: 0x002a_2a38,
            text_primary: 0x00f2_f2f7,
            text_secondary: 0x00b8_b8c8,
            text_tertiary: 0x0070_7080,
            attention_error: 0x00ff_453a,
            attention_input: 0x00ff_9f0a,
            success: 0x0030_d158,
            accent: 0x000a_84ff,
            selection: 0x0033_66cc,
        }
    }
}

impl Theme {
    /// The colour a cell's foreground takes when the window is not focused.
    pub fn inactive(self, color: u32) -> u32 {
        let r = ((color >> 16) & 0xff) / 2;
        let g = ((color >> 8) & 0xff) / 2;
        let b = (color & 0xff) / 2;
        (r << 16) | (g << 8) | b
    }

    /// The colour for an agent state, matching `SidebarView.stateColor`.
    pub fn agent_state_color(self, state: &str) -> u32 {
        match state {
            "blocked" | "waiting_input" => self.attention_input,
            "errored" => self.attention_error,
            "done" => self.success,
            "unknown" => self.text_tertiary,
            _ => self.accent,
        }
    }

    /// The glyph a provider uses in the sidebar.
    pub fn provider_glyph(provider: &str) -> &'static str {
        match provider {
            "claude-code" => "✦",
            "codex" => "⌬",
            "opencode" => "◇",
            _ => "•",
        }
    }

    /// The one-word state the sidebar prints.
    pub fn agent_state_label(state: &str) -> &'static str {
        match state {
            "thinking" => "thinking",
            "tool" => "tool",
            "waiting_input" => "waiting",
            "blocked" => "needs input",
            "done" => "done",
            "errored" => "error",
            "idle" => "idle",
            _ => "detected",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_an_error_state_when_colored_then_it_uses_the_attention_color() {
        let theme = Theme::default();
        assert_eq!(theme.agent_state_color("errored"), theme.attention_error);
        assert_eq!(
            theme.agent_state_color("waiting_input"),
            theme.attention_input
        );
        assert_eq!(theme.agent_state_color("thinking"), theme.accent);
    }

    #[test]
    fn given_each_provider_when_labelled_then_it_gets_the_mac_glyph() {
        assert_eq!(Theme::provider_glyph("claude-code"), "✦");
        assert_eq!(Theme::provider_glyph("codex"), "⌬");
        assert_eq!(Theme::provider_glyph("opencode"), "◇");
        assert_eq!(Theme::provider_glyph("other"), "•");
    }

    #[test]
    fn given_a_color_when_dimmed_then_it_keeps_its_hue() {
        let theme = Theme::default();
        let dimmed = theme.inactive(0x00ff_8040);
        assert_eq!((dimmed >> 16) & 0xff, 0x7f);
        assert_eq!((dimmed >> 8) & 0xff, 0x40);
        assert_eq!(dimmed & 0xff, 0x20);
    }
}
