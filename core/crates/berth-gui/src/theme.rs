//! Visual tokens for the portable window.
//!
//! The dark values mirror the macOS shell's `Theme.swift`: the system accent for
//! focus and the attention colours whose meaning the core already defines.
//! Views read these instead of hard-coding colours.

/// Palette choices, in the order the settings row cycles them. `system` is
/// not a palette of its own: it follows the OS light/dark appearance.
pub const THEMES: [&str; 3] = ["system", "berth-dark", "berth-light"];

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
    pub panel_bg: u32,
    pub panel_title_bg: u32,
    pub scrollbar: u32,
    pub banner_bg: u32,
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
            panel_bg: 0x0020_2028,
            panel_title_bg: 0x0029_2934,
            scrollbar: 0x001c_1c24,
            banner_bg: 0x0024_1418,
        }
    }
}

impl Theme {
    /// The palette `config.json` names. Unknown names fall back to the dark
    /// default, so a typo cannot leave the window unstyled.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "berth-dark" => Some(Self::default()),
            "berth-light" => Some(Self::light()),
            _ => None,
        }
    }

    /// The palette a config name selects. An empty name and `system` follow
    /// `system_light`; a named palette wins; an unknown name keeps the dark
    /// default so a typo cannot leave the window unstyled.
    pub fn resolve(name: &str, system_light: bool) -> Self {
        match name.trim() {
            "" | "system" => {
                if system_light {
                    Self::light()
                } else {
                    Self::default()
                }
            }
            other => Self::named(other).unwrap_or_default(),
        }
    }

    /// A light palette for daylight and projectors.
    fn light() -> Self {
        Self {
            terminal_bg: 0x00fb_fbfd,
            terminal_fg: 0x001c_1c22,
            pane_border: 0x00d9_d9e3,
            pane_border_focused: 0x000a_84ff,
            tab_bar_bg: 0x00ee_f0f4,
            tab_active_bg: 0x00ff_ffff,
            tab_text: 0x0026_2830,
            tab_text_dim: 0x0072_747e,
            sidebar_bg: 0x00f2_f3f7,
            sidebar_active: 0x00e0_e2ea,
            text_primary: 0x0014_171d,
            text_secondary: 0x0048_4c56,
            text_tertiary: 0x0080_868f,
            attention_error: 0x00d7_0015,
            attention_input: 0x00c2_5e00,
            success: 0x0024_8030,
            accent: 0x000a_84ff,
            selection: 0x00b6_d3ff,
            panel_bg: 0x00ff_ffff,
            panel_title_bg: 0x00ee_f0f4,
            scrollbar: 0x00df_e1e8,
            banner_bg: 0x00fd_ecee,
        }
    }

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
            "command-code" => "⌘",
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
        assert_eq!(Theme::provider_glyph("command-code"), "⌘");
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

    #[test]
    fn given_every_named_palette_then_it_resolves() {
        for name in ["berth-dark", "berth-light"] {
            assert!(Theme::named(name).is_some(), "{name} did not resolve");
        }
        assert!(THEMES.contains(&"system"));
    }

    #[test]
    fn given_the_system_choice_then_it_follows_the_appearance() {
        assert_eq!(
            Theme::resolve("", true).terminal_bg,
            Theme::named("berth-light").unwrap().terminal_bg
        );
        assert_eq!(
            Theme::resolve("system", false).terminal_bg,
            Theme::default().terminal_bg
        );
        assert_eq!(
            Theme::resolve("berth-dark", true).terminal_bg,
            Theme::default().terminal_bg,
            "an explicit palette must override the OS"
        );
    }

    #[test]
    fn given_an_unknown_theme_name_then_it_falls_back_to_the_default() {
        assert!(Theme::named("no-such-theme").is_none());
        assert_eq!(
            Theme::named("no-such-theme")
                .unwrap_or_default()
                .terminal_bg,
            Theme::default().terminal_bg
        );
    }

    #[test]
    fn given_the_light_theme_then_its_background_is_brighter_than_the_dark_one() {
        let light = Theme::named("berth-light").expect("light theme");
        assert!(luminance(light.terminal_bg) > luminance(Theme::default().terminal_bg));
        assert!(luminance(light.text_primary) < luminance(light.terminal_bg));
        assert!(luminance(Theme::default().text_primary) > luminance(Theme::default().terminal_bg));
    }

    fn luminance(color: u32) -> u32 {
        ((color >> 16) & 0xff) * 2 + ((color >> 8) & 0xff) * 3 + (color & 0xff)
    }
}
