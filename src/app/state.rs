use std::time::Duration;

use crate::config::{
    Keybinds, NewTerminalCwdConfig, PaneHeaderConfig, SoundConfig, ToastConfig, ToastDelivery,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Direction, Rect};
use ratatui::style::Color;

use crate::detect::AgentState;
use crate::layout::{PaneId, PaneInfo, SplitBorder};
use crate::selection::Selection;

// ---------------------------------------------------------------------------
// Selection autoscroll types
// ---------------------------------------------------------------------------

/// Direction of automatic scrolling during text selection drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionAutoscrollDirection {
    Up,
    Down,
}

/// State for automatic scrolling during text selection drag.
///
/// When the cursor hovers in the 1-row hot zone at the top or bottom edge
/// of a pane (or outside the pane), this struct captures the direction and
/// last known mouse position so a recurring 30ms tick can continue scrolling
/// and extending the selection even when the mouse is not moving.
#[derive(Clone, Debug)]
pub(crate) struct SelectionAutoscroll {
    pub direction: SelectionAutoscrollDirection,
    pub last_mouse_screen_col: u16,
    pub last_mouse_screen_row: u16,
    pub inner_rect: Rect,
}

#[derive(Clone)]
pub(crate) struct RightClickPassthroughGesture {
    pub pane_info: PaneInfo,
    pub modifiers: KeyModifiers,
}
use crate::terminal_theme::TerminalTheme;
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Theme palette — all UI colors in one place, ready for theming
// ---------------------------------------------------------------------------

/// All colors used by the UI. Derived from a base accent color for now,
/// but structured so a full theme system can replace it later.
#[derive(Clone)]
#[allow(dead_code)] // all fields defined for theming — some used later
pub struct Palette {
    /// Primary accent (tabs, highlights, modal chrome).
    pub accent: Color,
    /// Focused pane border. Falls back to [`Self::accent`] when unset.
    pub focus: Option<Color>,
    /// Background for floating panels, overlays, and modals.
    pub panel_bg: Color,
    /// Subtle surface background for selected/focused items.
    pub surface0: Color,
    /// Slightly lighter surface for hover/active states.
    pub surface1: Color,
    /// Very dim surface for separators.
    pub surface_dim: Color,
    /// Muted text (secondary info, numbers).
    pub overlay0: Color,
    /// Slightly brighter overlay text.
    pub overlay1: Color,
    /// Main text color — soft white.
    pub text: Color,
    /// Subdued text (workspace numbers, dim labels).
    pub subtext0: Color,
    /// Branch name / special label color.
    pub mauve: Color,
    /// Done / idle states.
    pub green: Color,
    /// Working / running states.
    pub yellow: Color,
    /// Needs attention / blocked states.
    pub red: Color,
    /// Unseen / done notification accent.
    pub blue: Color,
    /// Notification accent / unseen markers.
    pub teal: Color,
    /// Interrupted / warning states.
    pub peach: Color,
}

impl Palette {
    /// Catppuccin Mocha — the default.
    pub fn catppuccin() -> Self {
        Self {
            accent: Color::Rgb(137, 180, 250), // blue
            focus: None,
            panel_bg: Color::Rgb(24, 24, 37),
            surface0: Color::Rgb(49, 50, 68),
            surface1: Color::Rgb(69, 71, 90),
            surface_dim: Color::Rgb(30, 30, 46),
            overlay0: Color::Rgb(108, 112, 134),
            overlay1: Color::Rgb(127, 132, 156),
            text: Color::Rgb(205, 214, 244),
            subtext0: Color::Rgb(166, 173, 200),
            mauve: Color::Rgb(203, 166, 247),
            green: Color::Rgb(166, 227, 161),
            yellow: Color::Rgb(249, 226, 175),
            red: Color::Rgb(243, 139, 168),
            blue: Color::Rgb(137, 180, 250),
            teal: Color::Rgb(148, 226, 213),
            peach: Color::Rgb(250, 179, 135),
        }
    }

    /// Catppuccin Latte — the light Catppuccin flavor.
    pub fn catppuccin_latte() -> Self {
        Self {
            accent: Color::Rgb(30, 102, 245),
            focus: None,
            panel_bg: Color::Rgb(239, 241, 245),
            surface0: Color::Rgb(204, 208, 218),
            surface1: Color::Rgb(188, 192, 204),
            surface_dim: Color::Rgb(230, 233, 239),
            overlay0: Color::Rgb(156, 160, 176),
            overlay1: Color::Rgb(140, 143, 161),
            text: Color::Rgb(76, 79, 105),
            subtext0: Color::Rgb(108, 111, 133),
            mauve: Color::Rgb(136, 57, 239),
            green: Color::Rgb(64, 160, 43),
            yellow: Color::Rgb(223, 142, 29),
            red: Color::Rgb(210, 15, 57),
            blue: Color::Rgb(30, 102, 245),
            teal: Color::Rgb(23, 146, 153),
            peach: Color::Rgb(254, 100, 11),
        }
    }

    /// Terminal 16-color theme.
    pub fn terminal() -> Self {
        Self {
            accent: Color::Blue,
            focus: None,
            panel_bg: Color::Reset,
            surface0: Color::Reset,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::White,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Gray,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }

    /// Tokyo Night — blue-purple aesthetic.
    pub fn tokyo_night() -> Self {
        Self {
            accent: Color::Rgb(122, 162, 247), // blue
            focus: None,
            panel_bg: Color::Rgb(26, 27, 38),
            surface0: Color::Rgb(36, 40, 59),
            surface1: Color::Rgb(65, 72, 104),
            surface_dim: Color::Rgb(26, 27, 38),
            overlay0: Color::Rgb(86, 95, 137),
            overlay1: Color::Rgb(105, 113, 150),
            text: Color::Rgb(192, 202, 245),
            subtext0: Color::Rgb(169, 177, 214),
            mauve: Color::Rgb(187, 154, 247),
            green: Color::Rgb(158, 206, 106),
            yellow: Color::Rgb(224, 175, 104),
            red: Color::Rgb(247, 118, 142),
            blue: Color::Rgb(122, 162, 247),
            teal: Color::Rgb(125, 207, 255),
            peach: Color::Rgb(255, 158, 100),
        }
    }

    /// Tokyo Night Day — the light Tokyo Night style.
    pub fn tokyo_night_day() -> Self {
        Self {
            accent: Color::Rgb(46, 125, 233),
            focus: None,
            panel_bg: Color::Rgb(225, 226, 231),
            surface0: Color::Rgb(196, 200, 218),
            surface1: Color::Rgb(168, 174, 203),
            surface_dim: Color::Rgb(210, 211, 218),
            overlay0: Color::Rgb(137, 144, 179),
            overlay1: Color::Rgb(104, 112, 154),
            text: Color::Rgb(55, 96, 191),
            subtext0: Color::Rgb(97, 114, 176),
            mauve: Color::Rgb(120, 71, 189),
            green: Color::Rgb(88, 117, 57),
            yellow: Color::Rgb(140, 108, 62),
            red: Color::Rgb(245, 42, 101),
            blue: Color::Rgb(46, 125, 233),
            teal: Color::Rgb(17, 140, 116),
            peach: Color::Rgb(177, 92, 0),
        }
    }

    /// Dracula — purple/pink/green.
    pub fn dracula() -> Self {
        Self {
            accent: Color::Rgb(189, 147, 249), // purple
            focus: None,
            panel_bg: Color::Rgb(40, 42, 54),
            surface0: Color::Rgb(68, 71, 90),
            surface1: Color::Rgb(98, 114, 164),
            surface_dim: Color::Rgb(40, 42, 54),
            overlay0: Color::Rgb(98, 114, 164),
            overlay1: Color::Rgb(130, 140, 180),
            text: Color::Rgb(248, 248, 242),
            subtext0: Color::Rgb(210, 210, 220),
            mauve: Color::Rgb(255, 121, 198), // pink
            green: Color::Rgb(80, 250, 123),
            yellow: Color::Rgb(241, 250, 140),
            red: Color::Rgb(255, 85, 85),
            blue: Color::Rgb(139, 233, 253), // cyan-ish
            teal: Color::Rgb(139, 233, 253),
            peach: Color::Rgb(255, 184, 108),
        }
    }

    /// Synthwave — dracula with violet surfaces, cyan tabs, and magenta accents.
    pub fn synthwave() -> Self {
        Self {
            accent: Color::Rgb(54, 249, 246),      // #36F9F6 cyan workspace tabs
            focus: Some(Color::Rgb(244, 69, 247)), // #F445F7 active pane borders
            panel_bg: Color::Rgb(40, 42, 54),
            surface0: Color::Rgb(68, 71, 90),
            surface1: Color::Rgb(140, 130, 201), // #8C82C9
            surface_dim: Color::Rgb(40, 42, 54),
            overlay0: Color::Rgb(140, 130, 201), // #8C82C9
            overlay1: Color::Rgb(172, 156, 217),
            text: Color::Rgb(248, 248, 242),
            subtext0: Color::Rgb(210, 210, 220),
            mauve: Color::Rgb(255, 121, 198), // pink
            green: Color::Rgb(80, 250, 123),
            yellow: Color::Rgb(241, 250, 140),
            red: Color::Rgb(255, 85, 85),
            blue: Color::Rgb(244, 69, 247), // #F445F7
            teal: Color::Rgb(54, 249, 246), // #36F9F6
            peach: Color::Rgb(255, 184, 108),
        }
    }

    /// Nord — frosty blue palette.
    pub fn nord() -> Self {
        Self {
            accent: Color::Rgb(136, 192, 208), // frost
            focus: None,
            panel_bg: Color::Rgb(46, 52, 64),
            surface0: Color::Rgb(59, 66, 82),
            surface1: Color::Rgb(67, 76, 94),
            surface_dim: Color::Rgb(46, 52, 64),
            overlay0: Color::Rgb(76, 86, 106),
            overlay1: Color::Rgb(100, 110, 130),
            text: Color::Rgb(236, 239, 244),
            subtext0: Color::Rgb(216, 222, 233),
            mauve: Color::Rgb(180, 142, 173),
            green: Color::Rgb(163, 190, 140),
            yellow: Color::Rgb(235, 203, 139),
            red: Color::Rgb(191, 97, 106),
            blue: Color::Rgb(129, 161, 193),
            teal: Color::Rgb(143, 188, 187),
            peach: Color::Rgb(208, 135, 112),
        }
    }

    /// Gruvbox Dark — warm retro palette.
    pub fn gruvbox() -> Self {
        Self {
            accent: Color::Rgb(215, 153, 33), // yellow
            focus: None,
            panel_bg: Color::Rgb(40, 40, 40),
            surface0: Color::Rgb(60, 56, 54),
            surface1: Color::Rgb(80, 73, 69),
            surface_dim: Color::Rgb(40, 40, 40),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(168, 153, 132),
            text: Color::Rgb(235, 219, 178),
            subtext0: Color::Rgb(213, 196, 161),
            mauve: Color::Rgb(211, 134, 155),
            green: Color::Rgb(184, 187, 38),
            yellow: Color::Rgb(250, 189, 47),
            red: Color::Rgb(251, 73, 52),
            blue: Color::Rgb(131, 165, 152),
            teal: Color::Rgb(142, 192, 124),
            peach: Color::Rgb(254, 128, 25),
        }
    }

    /// Gruvbox Light — the light retro palette.
    pub fn gruvbox_light() -> Self {
        Self {
            accent: Color::Rgb(7, 102, 120),
            focus: None,
            panel_bg: Color::Rgb(251, 241, 199),
            surface0: Color::Rgb(235, 219, 178),
            surface1: Color::Rgb(213, 196, 161),
            surface_dim: Color::Rgb(242, 229, 188),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(124, 111, 100),
            text: Color::Rgb(60, 56, 54),
            subtext0: Color::Rgb(80, 73, 69),
            mauve: Color::Rgb(143, 63, 113),
            green: Color::Rgb(121, 116, 14),
            yellow: Color::Rgb(181, 118, 20),
            red: Color::Rgb(157, 0, 6),
            blue: Color::Rgb(7, 102, 120),
            teal: Color::Rgb(66, 123, 88),
            peach: Color::Rgb(175, 58, 3),
        }
    }

    /// One Dark — Atom's classic dark theme.
    pub fn one_dark() -> Self {
        Self {
            accent: Color::Rgb(97, 175, 239), // blue
            focus: None,
            panel_bg: Color::Rgb(40, 44, 52),
            surface0: Color::Rgb(44, 49, 58),
            surface1: Color::Rgb(62, 68, 81),
            surface_dim: Color::Rgb(40, 44, 52),
            overlay0: Color::Rgb(92, 99, 112),
            overlay1: Color::Rgb(115, 122, 135),
            text: Color::Rgb(171, 178, 191),
            subtext0: Color::Rgb(150, 156, 168),
            mauve: Color::Rgb(198, 120, 221),
            green: Color::Rgb(152, 195, 121),
            yellow: Color::Rgb(229, 192, 123),
            red: Color::Rgb(224, 108, 117),
            blue: Color::Rgb(97, 175, 239),
            teal: Color::Rgb(86, 182, 194),
            peach: Color::Rgb(209, 154, 102),
        }
    }

    /// One Light — Atom's classic light theme.
    pub fn one_light() -> Self {
        Self {
            accent: Color::Rgb(64, 120, 242),
            focus: None,
            panel_bg: Color::Rgb(250, 250, 250),
            surface0: Color::Rgb(240, 240, 241),
            surface1: Color::Rgb(229, 229, 230),
            surface_dim: Color::Rgb(245, 245, 246),
            overlay0: Color::Rgb(160, 161, 167),
            overlay1: Color::Rgb(104, 107, 119),
            text: Color::Rgb(56, 58, 66),
            subtext0: Color::Rgb(104, 107, 119),
            mauve: Color::Rgb(166, 38, 164),
            green: Color::Rgb(80, 161, 79),
            yellow: Color::Rgb(193, 132, 1),
            red: Color::Rgb(228, 86, 73),
            blue: Color::Rgb(64, 120, 242),
            teal: Color::Rgb(1, 132, 188),
            peach: Color::Rgb(152, 104, 1),
        }
    }

    /// Solarized Dark — Ethan Schoonover's classic.
    pub fn solarized() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210), // blue
            focus: None,
            panel_bg: Color::Rgb(0, 43, 54),
            surface0: Color::Rgb(7, 54, 66),
            surface1: Color::Rgb(88, 110, 117),
            surface_dim: Color::Rgb(0, 43, 54),
            overlay0: Color::Rgb(88, 110, 117),
            overlay1: Color::Rgb(101, 123, 131),
            text: Color::Rgb(147, 161, 161),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Solarized Light — Ethan Schoonover's light variant.
    pub fn solarized_light() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210),
            focus: None,
            panel_bg: Color::Rgb(253, 246, 227),
            surface0: Color::Rgb(238, 232, 213),
            surface1: Color::Rgb(147, 161, 161),
            surface_dim: Color::Rgb(238, 232, 213),
            overlay0: Color::Rgb(147, 161, 161),
            overlay1: Color::Rgb(88, 110, 117),
            text: Color::Rgb(101, 123, 131),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Kanagawa — inspired by Katsushika Hokusai.
    pub fn kanagawa() -> Self {
        Self {
            accent: Color::Rgb(126, 156, 216), // blue
            focus: None,
            panel_bg: Color::Rgb(31, 31, 40),
            surface0: Color::Rgb(42, 42, 55),
            surface1: Color::Rgb(54, 54, 70),
            surface_dim: Color::Rgb(31, 31, 40),
            overlay0: Color::Rgb(114, 113, 105),
            overlay1: Color::Rgb(135, 134, 125),
            text: Color::Rgb(220, 215, 186),
            subtext0: Color::Rgb(200, 195, 170),
            mauve: Color::Rgb(149, 127, 184),
            green: Color::Rgb(118, 148, 106),
            yellow: Color::Rgb(192, 163, 110),
            red: Color::Rgb(195, 64, 67),
            blue: Color::Rgb(126, 156, 216),
            teal: Color::Rgb(127, 180, 202),
            peach: Color::Rgb(255, 160, 102),
        }
    }

    /// Kanagawa Lotus — the light Kanagawa variant.
    pub fn kanagawa_lotus() -> Self {
        Self {
            accent: Color::Rgb(77, 105, 155),
            focus: None,
            panel_bg: Color::Rgb(242, 236, 188),
            surface0: Color::Rgb(220, 213, 172),
            surface1: Color::Rgb(201, 203, 209),
            surface_dim: Color::Rgb(213, 206, 163),
            overlay0: Color::Rgb(160, 156, 172),
            overlay1: Color::Rgb(138, 137, 128),
            text: Color::Rgb(84, 84, 100),
            subtext0: Color::Rgb(67, 67, 108),
            mauve: Color::Rgb(98, 76, 131),
            green: Color::Rgb(111, 137, 78),
            yellow: Color::Rgb(119, 113, 63),
            red: Color::Rgb(200, 64, 83),
            blue: Color::Rgb(77, 105, 155),
            teal: Color::Rgb(78, 140, 162),
            peach: Color::Rgb(204, 109, 0),
        }
    }

    /// Rosé Pine — muted, elegant.
    pub fn rose_pine() -> Self {
        Self {
            accent: Color::Rgb(196, 167, 231), // iris
            focus: None,
            panel_bg: Color::Rgb(25, 23, 36),
            surface0: Color::Rgb(31, 29, 46),
            surface1: Color::Rgb(38, 35, 58),
            surface_dim: Color::Rgb(25, 23, 36),
            overlay0: Color::Rgb(110, 106, 134),
            overlay1: Color::Rgb(144, 140, 170),
            text: Color::Rgb(224, 222, 244),
            subtext0: Color::Rgb(200, 197, 220),
            mauve: Color::Rgb(196, 167, 231),  // iris
            green: Color::Rgb(49, 116, 143),   // pine
            yellow: Color::Rgb(246, 193, 119), // gold
            red: Color::Rgb(235, 111, 146),    // love
            blue: Color::Rgb(49, 116, 143),    // pine
            teal: Color::Rgb(156, 207, 216),   // foam
            peach: Color::Rgb(234, 154, 151),  // rose
        }
    }

    /// Rosé Pine Dawn — the light Rosé Pine variant.
    pub fn rose_pine_dawn() -> Self {
        Self {
            accent: Color::Rgb(144, 122, 169),
            focus: None,
            panel_bg: Color::Rgb(250, 244, 237),
            surface0: Color::Rgb(242, 233, 225),
            surface1: Color::Rgb(255, 250, 243),
            surface_dim: Color::Rgb(242, 233, 225),
            overlay0: Color::Rgb(152, 147, 165),
            overlay1: Color::Rgb(121, 117, 147),
            text: Color::Rgb(70, 66, 97),
            subtext0: Color::Rgb(121, 117, 147),
            mauve: Color::Rgb(144, 122, 169),
            green: Color::Rgb(40, 105, 131),
            yellow: Color::Rgb(234, 157, 52),
            red: Color::Rgb(180, 99, 122),
            blue: Color::Rgb(40, 105, 131),
            teal: Color::Rgb(86, 148, 159),
            peach: Color::Rgb(215, 130, 126),
        }
    }

    /// Vesper — minimal high-contrast monochrome with peach and mint accents.
    pub fn vesper() -> Self {
        Self {
            accent: Color::Rgb(255, 199, 153),
            focus: None,
            panel_bg: Color::Rgb(26, 26, 26),
            surface0: Color::Rgb(35, 35, 35),
            surface1: Color::Rgb(40, 40, 40),
            surface_dim: Color::Rgb(16, 16, 16),
            overlay0: Color::Rgb(92, 92, 92),
            overlay1: Color::Rgb(126, 126, 126),
            text: Color::Rgb(255, 255, 255),
            subtext0: Color::Rgb(160, 160, 160),
            mauve: Color::Rgb(255, 209, 168),
            green: Color::Rgb(153, 255, 228),
            yellow: Color::Rgb(255, 199, 153),
            red: Color::Rgb(255, 128, 128),
            blue: Color::Rgb(176, 176, 176),
            teal: Color::Rgb(102, 221, 204),
            peach: Color::Rgb(255, 199, 153),
        }
    }

    /// Resolve a theme by name. Returns None for unknown names.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().replace([' ', '_'], "-").as_str() {
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin()),
            "catppuccin-latte" | "latte" | "light" => Some(Self::catppuccin_latte()),
            "terminal" => Some(Self::terminal()),
            "tokyo-night" | "tokyonight" => Some(Self::tokyo_night()),
            "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => Some(Self::tokyo_night_day()),
            "dracula" => Some(Self::dracula()),
            "synthwave" => Some(Self::synthwave()),
            "nord" => Some(Self::nord()),
            "gruvbox" | "gruvbox-dark" => Some(Self::gruvbox()),
            "gruvbox-light" => Some(Self::gruvbox_light()),
            "one-dark" | "onedark" => Some(Self::one_dark()),
            "one-light" | "onelight" => Some(Self::one_light()),
            "solarized" | "solarized-dark" => Some(Self::solarized()),
            "solarized-light" => Some(Self::solarized_light()),
            "kanagawa" => Some(Self::kanagawa()),
            "kanagawa-lotus" | "lotus" => Some(Self::kanagawa_lotus()),
            "rose-pine" | "rosepine" => Some(Self::rose_pine()),
            "rose-pine-dawn" | "rosepine-dawn" | "dawn" => Some(Self::rose_pine_dawn()),
            "vesper" => Some(Self::vesper()),
            _ => None,
        }
    }

    /// Apply custom color overrides on top of this palette.
    pub fn with_overrides(mut self, custom: &crate::config::CustomThemeColors) -> Self {
        use crate::config::parse_color;
        if let Some(c) = &custom.accent {
            self.accent = parse_color(c);
        }
        if let Some(c) = &custom.focus {
            self.focus = Some(parse_color(c));
        }
        if let Some(c) = &custom.panel_bg {
            self.panel_bg = parse_color(c);
        }
        if let Some(c) = &custom.surface0 {
            self.surface0 = parse_color(c);
        }
        if let Some(c) = &custom.surface1 {
            self.surface1 = parse_color(c);
        }
        if let Some(c) = &custom.surface_dim {
            self.surface_dim = parse_color(c);
        }
        if let Some(c) = &custom.overlay0 {
            self.overlay0 = parse_color(c);
        }
        if let Some(c) = &custom.overlay1 {
            self.overlay1 = parse_color(c);
        }
        if let Some(c) = &custom.text {
            self.text = parse_color(c);
        }
        if let Some(c) = &custom.subtext0 {
            self.subtext0 = parse_color(c);
        }
        if let Some(c) = &custom.mauve {
            self.mauve = parse_color(c);
        }
        if let Some(c) = &custom.green {
            self.green = parse_color(c);
        }
        if let Some(c) = &custom.yellow {
            self.yellow = parse_color(c);
        }
        if let Some(c) = &custom.red {
            self.red = parse_color(c);
        }
        if let Some(c) = &custom.blue {
            self.blue = parse_color(c);
        }
        if let Some(c) = &custom.teal {
            self.teal = parse_color(c);
        }
        if let Some(c) = &custom.peach {
            self.peach = parse_color(c);
        }
        self
    }

    /// Border color for the focused pane.
    pub fn focused_pane_border(&self) -> Color {
        self.focus.unwrap_or(self.accent)
    }

    /// Dim border for shared edges on unfocused panes.
    pub fn dim_pane_border(&self) -> Color {
        self.panel_bg
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreateState {
    pub source_workspace_id: String,
    pub source_checkout_path: std::path::PathBuf,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub branch: String,
    pub checkout_path: std::path::PathBuf,
    pub error: Option<String>,
    pub creating: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRemoveState {
    pub workspace_id: String,
    pub pane_id: Option<PaneId>,
    pub repo_root: std::path::PathBuf,
    pub path: std::path::PathBuf,
    pub error: Option<String>,
    pub removing: bool,
    pub force_confirmation: bool,
    pub already_landed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeLandState {
    pub workspace_id: String,
    pub path: std::path::PathBuf,
    pub label: String,
    pub parent_branch: String,
    pub landing: bool,
    pub error: Option<String>,
    pub result_title: Option<String>,
    pub result_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenEntry {
    pub path: std::path::PathBuf,
    pub branch: Option<String>,
    pub is_linked_worktree: bool,
    pub already_open_ws_idx: Option<usize>,
}

impl WorktreeOpenEntry {
    pub(crate) fn display_name(&self) -> String {
        self.branch.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if self.already_open_ws_idx.is_some() {
            "open"
        } else if self.branch.is_some() {
            ""
        } else if self.is_linked_worktree {
            "detached"
        } else {
            "root"
        }
    }

    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.display_name(),
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
            self.path.display(),
            self.status_label()
        )
        .to_lowercase()
    }

    fn matches_query(&self, query: &str) -> bool {
        text_matches_query(query, &self.search_text())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenState {
    pub source_workspace_id: String,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_checkout_path: std::path::PathBuf,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub entries: Vec<WorktreeOpenEntry>,
    pub selected: usize,
    pub query: String,
    pub search_focused: bool,
    pub error: Option<String>,
}

impl WorktreeOpenState {
    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim();
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                (query.is_empty() || entry.matches_query(query)).then_some(idx)
            })
            .collect()
    }

    pub(crate) fn selected_entry_index(&self) -> Option<usize> {
        let indices = self.filtered_indices();
        if indices.contains(&self.selected) {
            Some(self.selected)
        } else {
            indices.first().copied()
        }
    }

    pub(crate) fn normalize_selection(&mut self) {
        if let Some(selected) = self.selected_entry_index() {
            self.selected = selected;
        }
    }

    pub(crate) fn select_previous_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[pos.saturating_sub(1)];
    }

    pub(crate) fn select_next_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[(pos + 1).min(indices.len().saturating_sub(1))];
    }
}

pub(crate) fn text_matches_query(query: &str, text: &str) -> bool {
    let haystack = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|needle| haystack.contains(needle))
}

/// Computed view geometry — derived from AppState + terminal size.
/// Updated before each render, consumed by render and mouse handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewLayout {
    Desktop,
    Mobile,
}

#[derive(Clone, Copy)]
pub struct PaneTitleHitArea {
    pub pane_id: PaneId,
    pub rect: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaneChromeAction {
    Focus,
    Close,
}

#[derive(Clone, Copy)]
pub struct PaneChromeControl {
    pub pane_id: PaneId,
    pub action: PaneChromeAction,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceCardArea {
    pub ws_idx: usize,
    pub rect: Rect,
    pub indented: bool,
}

/// Clickable region for an agent row nested under its space card. The rect
/// covers only the entry's content rows, not the leading gap row and not the
/// folder header row above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRowArea {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: PaneId,
    pub rect: Rect,
    /// Whether the row directly above this one is the folder header this agent
    /// heads. Agents listed under it in the same folder share that one header,
    /// so only the first of them carries it.
    pub location_header: bool,
}

/// The folder row a space's agents are listed under. It is a label rather than
/// a card — clicking it selects nothing — but dragging it moves the whole
/// folder among its space's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFolderArea {
    pub ws_idx: usize,
    /// The folder itself, which is the identity a drag carries.
    pub key: String,
    /// The first agent listed under it, whose location supplies the row's text.
    pub tab_idx: usize,
    pub pane_id: PaneId,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarWidthSource {
    ConfigDefault,
    Persisted,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum AgentPanelScope {
    CurrentWorkspace,
    #[default]
    AllWorkspaces,
}

pub struct ViewState {
    pub layout: ViewLayout,
    /// The rows the composer occupies above every other surface, and where its
    /// controls sit on them.
    pub composer: crate::ui::ComposerLayout,
    /// Left column of space cards and the agents under them.
    pub sidebar_rect: Rect,
    pub workspace_card_areas: Vec<WorkspaceCardArea>,
    pub agent_row_areas: Vec<AgentRowArea>,
    pub agent_folder_areas: Vec<AgentFolderArea>,
    /// The agent table between the composer and the panes: where its rows and
    /// columns sit, and which agent each row is.
    pub agent_table: crate::ui::AgentTableLayout,
    /// Where each listed pane is working, as of the last frame. The table is
    /// laid out from `AppState` alone, which cannot see the live runtimes a
    /// pane's current folder comes from, so it reads that folder from here —
    /// and so does the paint that writes the row.
    pub agent_locations: std::collections::HashMap<PaneId, crate::ui::AgentLocation>,
    pub terminal_area: Rect,
    pub mobile_header_rect: Rect,
    pub mobile_menu_hit_area: Rect,
    pub toast_hit_area: Rect,
    pub pane_infos: Vec<PaneInfo>,
    pub pane_chrome_controls: Vec<PaneChromeControl>,
    pub pane_title_hit_areas: Vec<PaneTitleHitArea>,
    pub split_borders: Vec<SplitBorder>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            layout: ViewLayout::Desktop,
            composer: crate::ui::ComposerLayout::default(),
            sidebar_rect: Rect::default(),
            workspace_card_areas: Vec::new(),
            agent_row_areas: Vec::new(),
            agent_folder_areas: Vec::new(),
            agent_table: crate::ui::AgentTableLayout::default(),
            agent_locations: std::collections::HashMap::new(),
            terminal_area: Rect::default(),
            mobile_header_rect: Rect::default(),
            mobile_menu_hit_area: Rect::default(),
            toast_hit_area: Rect::default(),
            pane_infos: Vec::new(),
            pane_chrome_controls: Vec::new(),
            pane_title_hit_areas: Vec::new(),
            split_borders: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Onboarding,
    ReleaseNotes,
    ProductAnnouncement,
    Navigate,
    Prefix,
    Copy,
    Terminal,
    Composer,
    RenameWorkspace,
    RenamePane,
    NewLinkedWorktree,
    OpenExistingWorktree,
    ConfirmRemoveWorktree,
    WorktreeLand,
    Resize,
    ConfirmClose,
    ConfirmCloseAgent,
    ContextMenu,
    Settings,
    GlobalMenu,
    KeybindHelp,
    Navigator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NavigatorTarget {
    Workspace {
        ws_idx: usize,
    },
    Tab {
        ws_idx: usize,
        tab_idx: usize,
    },
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigatorRow {
    pub target: NavigatorTarget,
    pub depth: u8,
    pub label: String,
    pub meta: String,
    pub status: AgentState,
    pub seen: bool,
    pub is_current: bool,
    pub is_workspace: bool,
    pub is_tab: bool,
    pub expanded: bool,
    pub search_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigatorStateFilter {
    Blocked,
    Working,
    Idle,
    Done,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NavigatorState {
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub search_focused: bool,
    pub state_filter: Option<NavigatorStateFilter>,
    pub expanded_workspaces: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyModeState {
    pub pane_id: PaneId,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub entry_offset_from_bottom: usize,
    pub selection: Option<CopyModeSelection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyModeSelection {
    Character,
    Linewise { anchor_row: u32 },
}

// ---------------------------------------------------------------------------
// Settings UI state
// ---------------------------------------------------------------------------

/// Which section of the settings panel is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
    Sound,
    Toast,
    PaneLabels,
    Experiments,
    Integrations,
}

impl SettingsSection {
    pub const ALL: &[Self] = &[
        Self::Theme,
        Self::Sound,
        Self::Toast,
        Self::PaneLabels,
        Self::Integrations,
        Self::Experiments,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Sound => "sound",
            Self::Toast => "toasts",
            Self::PaneLabels => "pane labels",
            Self::Experiments => "experiments",
            Self::Integrations => "integrations",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExperimentSetting {
    PaneHistory,
    SwitchAsciiInputSourceInPrefix,
    RefreshSummaryWithGrok,
}

impl ExperimentSetting {
    pub(crate) const ALL: [Self; 3] = [
        Self::PaneHistory,
        Self::SwitchAsciiInputSourceInPrefix,
        Self::RefreshSummaryWithGrok,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PaneHistory => "pane screen history",
            Self::SwitchAsciiInputSourceInPrefix => {
                "switch to ascii input source in prefix (macOS)"
            }
            Self::RefreshSummaryWithGrok => "refresh summary with grok",
        }
    }

    pub(crate) fn enabled(self, state: &AppState) -> bool {
        match self {
            Self::PaneHistory => state.pane_history_persistence_enabled(),
            Self::SwitchAsciiInputSourceInPrefix => {
                state.switch_ascii_input_source_in_prefix_enabled()
            }
            Self::RefreshSummaryWithGrok => state.refresh_summary_with_grok(),
        }
    }
}

/// All built-in theme names in display order.
pub const THEME_NAMES: &[&str] = &[
    "catppuccin",
    "catppuccin-latte",
    "terminal",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "synthwave",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "one-dark",
    "one-light",
    "solarized",
    "solarized-light",
    "kanagawa",
    "kanagawa-lotus",
    "rose-pine",
    "rose-pine-dawn",
    "vesper",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuListState {
    pub highlighted: usize,
}

impl MenuListState {
    pub fn new(highlighted: usize) -> Self {
        Self { highlighted }
    }

    pub fn move_prev(&mut self) {
        self.highlighted = self.highlighted.saturating_sub(1);
    }

    pub fn move_next(&mut self, item_count: usize) {
        if item_count > 0 {
            self.highlighted = (self.highlighted + 1).min(item_count - 1);
        }
    }

    pub fn hover(&mut self, idx: Option<usize>) {
        if let Some(idx) = idx {
            self.highlighted = idx;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionListState {
    pub selected: usize,
}

impl SelectionListState {
    pub fn new(selected: usize) -> Self {
        Self { selected }
    }

    pub fn move_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_next(&mut self, item_count: usize) {
        if item_count > 0 {
            self.selected = (self.selected + 1).min(item_count - 1);
        }
    }

    pub fn select(&mut self, idx: usize) {
        self.selected = idx;
    }
}

/// One row in the settings "done sound" picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoneSoundChoice {
    /// The mp3 configured through `ui.sound.done_path` or `ui.sound.path`.
    CustomFile(std::path::PathBuf),
    /// A sound shipped inside the binary.
    Builtin(&'static crate::sound::BuiltinSound),
}

impl DoneSoundChoice {
    pub fn label(&self) -> String {
        match self {
            Self::CustomFile(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string()),
            Self::Builtin(sound) => sound.key.to_string(),
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::CustomFile(_) => "your own mp3, set in the config file",
            Self::Builtin(sound) => sound.description,
        }
    }
}

pub struct SettingsState {
    /// Which section tab is active.
    pub section: SettingsSection,
    /// Selected item index within the current section.
    pub list: SelectionListState,
    /// The palette before opening settings (for cancel/restore).
    pub original_palette: Option<Palette>,
    /// The theme name before opening settings.
    pub original_theme: Option<String>,
    /// The Refresh Summary prompt field is focused for typing.
    pub editing_refresh_prompt: bool,
}

pub(crate) enum DragTarget {
    WorkspaceReorder {
        source_ws_idx: usize,
        insert_idx: Option<usize>,
    },
    /// Reordering an agent row among the agents it shares a folder with in
    /// the sidebar. Display order only — the pane layout is untouched.
    /// Hovering `+ new` instead flies the pane out into its own space.
    SidebarAgentReorder {
        ws_idx: usize,
        source_pane_id: PaneId,
        insert_idx: Option<usize>,
        create_space: bool,
    },
    /// Reordering a whole folder, and every agent under it, among its space's
    /// folders. Display order only.
    AgentFolderReorder {
        ws_idx: usize,
        key: String,
        insert_idx: Option<usize>,
    },
    WorkspaceListScrollbar {
        grab_row_offset: u16,
    },
    SidebarDivider,
    /// Reordering the session-wide agent table. This changes presentation
    /// order only; pane placement and workspace membership stay untouched,
    /// except hovering `+ new` flies a docked pane out into its own space.
    AgentReorder {
        source_pane_id: PaneId,
        insert_idx: Option<usize>,
        create_space: bool,
    },
    /// Carrying a set-down agent out of the table. Dropped against a pane's
    /// edge it cuts that pane in two and docks there; dropped over the middle
    /// it takes the pane whole; dropped on `+ new` it becomes its own space.
    AgentDock {
        pane_id: PaneId,
        hovered_pane_id: Option<PaneId>,
        drop_zone: crate::layout::DropZone,
        create_space: bool,
    },
    PaneSplit {
        path: Vec<bool>,
        direction: Direction,
        area: Rect,
    },
    /// Carrying a pane across the panes. Dropped over the middle of another it
    /// trades places with it; dropped against one of that pane's edges it cuts
    /// the pane in two and takes the half against that edge.
    PaneSwap {
        source_pane_id: PaneId,
        hovered_pane_id: Option<PaneId>,
        drop_zone: crate::layout::DropZone,
        /// Hovering the sidebar `+ new` button, which flies the pane out into
        /// its own space.
        create_space: bool,
        moved: bool,
    },
    PaneScrollbar {
        pane_id: crate::layout::PaneId,
        grab_row_offset: u16,
    },
    ReleaseNotesScrollbar {
        grab_row_offset: u16,
    },
    ProductAnnouncementScrollbar {
        grab_row_offset: u16,
    },
    KeybindHelpScrollbar {
        grab_row_offset: u16,
    },
}

/// Active mouse drag on a split border, a pane, or a scrollbar.
pub(crate) struct DragState {
    pub target: DragTarget,
}

/// A pane set down off every layout, with its running agent still inside.
///
/// It keeps its pane id because the id is the wiring: the agent's terminal has
/// sent its events under this id since it was spawned, and docking the agent
/// puts the same id back into a layout, so nothing has to be re-routed.
pub struct DetachedAgent {
    pub pane_id: PaneId,
    pub pane: crate::pane::PaneState,
}

/// Where a set-down agent is working: the live cwd of its terminal, falling back
/// to the directory it was launched in. This is what [`crate::workspace::Tab`]
/// answers for a docked pane, for an agent no tab holds.
pub(crate) fn detached_agent_cwd(
    pane: &crate::pane::PaneState,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
) -> Option<std::path::PathBuf> {
    let terminal_id = &pane.attached_terminal_id;
    terminal_runtimes
        .get(terminal_id)
        .and_then(|runtime| runtime.cwd())
        .or_else(|| {
            terminals
                .get(terminal_id)
                .map(|terminal| terminal.cwd.clone())
        })
}

pub(crate) struct WorkspacePressState {
    pub ws_idx: usize,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct PanePressState {
    pub pane_id: PaneId,
    pub start_col: u16,
    pub start_row: u16,
}

/// Left button held on a sidebar agent row, waiting to become a reorder drag.
pub(crate) struct SidebarAgentPressState {
    pub ws_idx: usize,
    pub pane_id: PaneId,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct AgentFolderPressState {
    pub ws_idx: usize,
    pub key: String,
    pub start_col: u16,
    pub start_row: u16,
}

/// The agent a click on the table just picked out, held until the next key.
/// The click focuses the agent's pane as it always has, so typing goes to the
/// agent; the hold only decides whether the delete key means this row or the
/// pane below it. Any key that is not delete releases it and goes on as usual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentTableFocus {
    pub docked: bool,
    pub pane_id: PaneId,
}

/// An agent the table has offered to remove, waiting on the second key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingAgentClose {
    pub docked: bool,
    pub pane_id: PaneId,
    /// What the row calls the agent, so the question names what it will end.
    pub name: String,
}

/// Left button held on a table row, waiting to become a drag. A docked row's
/// drag carries its pane; a set-down row's drag carries the agent itself.
pub(crate) struct AgentPressState {
    pub docked: bool,
    pub pane_id: PaneId,
    pub start_col: u16,
    pub start_row: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuKind {
    Workspace {
        ws_idx: usize,
    },
    GitWorkspace {
        ws_idx: usize,
        is_linked_worktree: bool,
        has_worktree_children: bool,
        collapsed: bool,
    },
    /// A row of the agent table. The row is the only handle on the space the
    /// agent works in as well as on the agent itself, so this menu carries
    /// what can be done to the agent, then the worktree actions its folder
    /// allows. A set-down agent uses the same menu: land and worktree
    /// deletion follow that agent's folder, not a space.
    Agent {
        ws_idx: usize,
        pane_id: PaneId,
        /// What the space offers, which depends on whether it is a Git
        /// checkout and whether that checkout is a linked worktree.
        space: SpaceMenuKind,
    },
    Pane {
        pane_id: PaneId,
        has_manual_label: bool,
        dimmed: bool,
        /// Whether an agent occupies the pane's terminal, which adds the
        /// option to close the agent while keeping the pane.
        has_agent: bool,
        /// Whether herdr knows the running agent's reset command, which adds
        /// the option to start a new session in place.
        can_reset: bool,
    },
}

/// What a space offers a menu, which is what its Git state allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceMenuKind {
    /// Not a Git checkout. Worktree deletion still sits at the bottom of the
    /// menu, grayed, because the agent is not in a linked worktree directory.
    Plain,
    /// A Git checkout worktrees can be made from.
    Repo,
    /// A linked worktree, which can be given up rather than added to.
    /// `parent_branch` is the branch currently checked out in the parent
    /// checkout, which is where land will go.
    /// `in_worktree_directory` is whether this agent's own directory is that
    /// linked checkout. Land and worktree deletion stay on the menu when the
    /// space is a worktree, and go gray when this agent is not in it.
    LinkedWorktree {
        parent_branch: Option<String>,
        already_landed: bool,
        in_worktree_directory: bool,
    },
}

pub fn land_menu_label(parent_branch: Option<&str>) -> String {
    match parent_branch {
        Some(branch) if !branch.is_empty() => format!("Land on {branch}"),
        _ => "Land on parent".to_string(),
    }
}

pub fn is_land_menu_item(item: &str) -> bool {
    item.starts_with("Land on ")
}

pub fn land_prompt_text() -> String {
    "Land this clean branch by running `herdr agent worktree land`".to_string()
}

/// Right-click context menu state.
pub struct ContextMenuState {
    pub kind: ContextMenuKind,
    pub x: u16,
    pub y: u16,
    pub list: MenuListState,
}

impl ContextMenuState {
    pub fn items(&self) -> Vec<String> {
        match &self.kind {
            ContextMenuKind::Workspace { .. } => vec!["Rename".into(), "Close".into()],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: false,
                has_worktree_children: false,
                ..
            } => vec![
                "Rename".into(),
                "Close".into(),
                "New worktree".into(),
                "Open worktree...".into(),
            ],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: true,
                ..
            } => vec![
                "Rename".into(),
                "Close".into(),
                "Delete worktree checkout...".into(),
            ],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed,
                ..
            } => vec![
                "Rename".into(),
                "Close group".into(),
                "New worktree".into(),
                "Open worktree...".into(),
                if *collapsed {
                    "Expand".into()
                } else {
                    "Collapse".into()
                },
            ],
            ContextMenuKind::Agent { space, .. } => {
                let mut items = vec![
                    "Rename agent".into(),
                    "Refresh Summary".into(),
                    "Delete agent".into(),
                ];
                match space {
                    SpaceMenuKind::Plain => {}
                    SpaceMenuKind::Repo => {
                        items.extend(["New worktree".into(), "Open worktree...".into()]);
                    }
                    SpaceMenuKind::LinkedWorktree { parent_branch, .. } => {
                        items.push(land_menu_label(parent_branch.as_deref()));
                    }
                }
                items.push("Delete agent + worktree".into());
                items
            }
            ContextMenuKind::Pane {
                has_manual_label,
                dimmed,
                has_agent,
                can_reset,
                ..
            } => {
                let mut items = vec!["Rename pane".into()];
                if *has_manual_label {
                    items.push("Clear pane name".into());
                }
                items.extend([
                    "Split vertically".into(),
                    "Split horizontally".into(),
                    "Zoom".into(),
                ]);
                items.push(if *dimmed {
                    "Undim".into()
                } else {
                    "Dim".into()
                });
                items.push("Close pane".into());
                if *can_reset {
                    items.push("Reset agent".into());
                }
                if *has_agent {
                    items.push("Close agent".into());
                }
                items
            }
        }
    }

    pub fn item_enabled(&self, idx: usize) -> bool {
        let items = self.items();
        let Some(item) = items.get(idx) else {
            return true;
        };
        match &self.kind {
            ContextMenuKind::Agent { space, .. } => {
                if is_land_menu_item(item) {
                    matches!(
                        space,
                        SpaceMenuKind::LinkedWorktree {
                            already_landed: false,
                            in_worktree_directory: true,
                            ..
                        }
                    )
                } else if item == "Delete agent + worktree" {
                    matches!(
                        space,
                        SpaceMenuKind::LinkedWorktree {
                            in_worktree_directory: true,
                            ..
                        }
                    )
                } else {
                    true
                }
            }
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    NeedsAttention,
    Finished,
    UpdateInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastNotification {
    pub kind: ToastKind,
    pub title: String,
    pub context: String,
    pub target: Option<ToastTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFeedback {
    pub message: String,
}

pub struct ReleaseNotesState {
    pub version: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct ProductAnnouncementState {
    pub version: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct KeybindHelpState {
    pub scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneFocusTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

/// Time window for the second Ctrl+Q press to confirm detach/exit.
pub(crate) const DETACH_CONFIRM_WINDOW: Duration = Duration::from_millis(500);

/// All application state — pure data, no channels or async runtime.
/// Testable without PTYs or a tokio runtime.
pub struct AppState {
    pub terminals:
        std::collections::HashMap<crate::terminal::TerminalId, crate::terminal::TerminalState>,
    /// Agents running with no pane showing them. Closing a pane sets its agent
    /// down here instead of killing it; the agent keeps its table row, and a
    /// drag from that row docks it into a pane again. Only "Delete agent" ends
    /// one: a docked agent loses its pane with it, and a set-down agent leaves
    /// this list.
    pub detached_agents: Vec<DetachedAgent>,
    /// The branch and worktree state of each set-down agent's directory. A
    /// docked pane inherits this from the space holding it; an agent no space
    /// holds keeps its own answer here.
    pub(crate) detached_git_statuses:
        std::collections::HashMap<PaneId, crate::workspace::WorkspaceGitStatusSnapshot>,
    /// Terminal ids whose size is currently owned by a direct attach client.
    pub direct_attach_resize_locks: std::collections::HashSet<crate::terminal::TerminalId>,
    pub(crate) pane_id_aliases: std::collections::HashMap<u32, PaneId>,
    pub workspaces: Vec<Workspace>,
    pub active: Option<usize>,
    pub(crate) previous_pane_focus: Option<PaneFocusTarget>,
    pub selected: usize,
    pub mode: Mode,
    pub should_quit: bool,
    /// In monolithic --no-session mode, detach exits the app because there is no server to detach from.
    pub detach_exits: bool,
    /// Set when the current client should detach from the persistent session.
    /// The server's event loop checks this and handles client detach.
    pub detach_requested: bool,
    /// Direct Ctrl+Q detach requires a second Ctrl+Q press within
    /// [`DETACH_CONFIRM_WINDOW`].
    pub detach_confirm_until: Option<std::time::Instant>,
    pub request_new_workspace: bool,
    pub request_new_tab: bool,
    pub request_new_linked_worktree: Option<usize>,
    pub request_open_existing_worktree: Option<usize>,
    pub request_new_workspace_cwd: Option<std::path::PathBuf>,
    pub request_remove_linked_worktree: Option<usize>,
    pub request_remove_agent_worktree: Option<PaneId>,
    pub request_submit_worktree_create: bool,
    pub request_submit_worktree_open: bool,
    pub request_submit_worktree_remove: bool,
    pub request_reload_config: bool,
    /// Set when the headless server should ask attached clients to reload
    /// their client-local sound config from disk.
    pub request_client_config_reload: bool,
    /// Set when UI interaction requested a clipboard write that must be
    /// handled by the outer App/event loop instead of directly from AppState.
    pub request_clipboard_write: Option<Vec<u8>>,
    pub creating_new_tab: bool,
    pub requested_new_tab_name: Option<String>,
    pub rename_pane_target: Option<PaneId>,
    pub worktree_create: Option<WorktreeCreateState>,
    pub worktree_open: Option<WorktreeOpenState>,
    pub worktree_remove: Option<WorktreeRemoveState>,
    pub worktree_land: Option<WorktreeLandState>,
    pub worktree_directory: String,
    pub worktree_verify_command: Vec<String>,
    pub worktree_auto_land: bool,
    pub request_land_worktree: Option<usize>,
    pub request_land_agent_prompt: Option<(String, String)>,
    pub request_refresh_summary: Option<PaneId>,
    pub landing_worktrees: std::collections::HashSet<String>,
    pub landing_failures: std::collections::HashMap<String, String>,
    pub request_complete_onboarding: bool,
    pub name_input: String,
    pub name_input_replace_on_type: bool,
    /// The always-visible band that starts agents: where to work, who works,
    /// what to do.
    pub composer: crate::composer::ComposerState,
    pub release_notes: Option<ReleaseNotesState>,
    pub product_announcement: Option<ProductAnnouncementState>,
    pub keybind_help: KeybindHelpState,
    pub navigator: NavigatorState,
    pub copy_mode: Option<CopyModeState>,
    /// Stable, session-wide order for agent rows. Terminal ids survive pane
    /// moves and session restore, unlike layout positions and pane ids.
    pub agent_order: Vec<crate::terminal::TerminalId>,
    pub collapsed_space_keys: std::collections::HashSet<String>,
    /// Ids of spaces whose agent entries are folded away in the sidebar.
    /// Spaces default to expanded, so only collapsed ones are tracked.
    pub collapsed_agent_space_ids: std::collections::HashSet<String>,
    pub workspace_scroll: usize,
    pub default_sidebar_width: u16,
    pub sidebar_width: u16,
    pub sidebar_min_width: u16,
    pub sidebar_max_width: u16,
    pub sidebar_width_source: SidebarWidthSource,
    #[allow(dead_code)]
    pub sidebar_width_auto: bool,
    pub sidebar_collapsed: bool,
    pub spaces_collapsed: bool,
    /// Legacy ratio of sidebar height once allocated to the workspaces
    /// section. Kept so older sessions still restore.
    pub sidebar_section_split: f32,
    #[allow(dead_code)]
    pub agent_panel_scope: AgentPanelScope,
    /// How far down the agent list the table's first drawn row sits. It follows
    /// the focused agent, and a wheel notch over the table moves it directly.
    pub agent_table_scroll: usize,
    pub mobile_switcher_scroll: usize,
    // View geometry (computed before render, consumed by render + mouse)
    pub view: ViewState,
    pub(crate) drag: Option<DragState>,
    pub(crate) workspace_press: Option<WorkspacePressState>,
    pub(crate) pane_press: Option<PanePressState>,
    pub(crate) agent_press: Option<AgentPressState>,
    pub(crate) sidebar_agent_press: Option<SidebarAgentPressState>,
    pub(crate) agent_folder_press: Option<AgentFolderPressState>,
    /// The row a click picked out, until the next key releases it.
    pub(crate) agent_table_focus: Option<AgentTableFocus>,
    /// An agent shown full-pane over the current layout. The splits underneath
    /// do not change, so a shell that was on screen keeps running. EXIT
    /// clears it. Clicking this row again leaves it showing.
    pub(crate) agent_peek: Option<PaneId>,
    /// The agent the delete key has asked about, until enter or escape answers.
    pub(crate) confirm_close_agent: Option<PendingAgentClose>,
    pub selection: Option<Selection>,
    pub selection_autoscroll: Option<SelectionAutoscroll>,
    pub context_menu: Option<ContextMenuState>,
    // Notifications
    pub update_available: Option<String>,
    pub update_install_command: String,
    pub latest_release_notes_available: bool,
    pub update_dismissed: bool,
    pub config_diagnostic: Option<String>,
    pub toast: Option<ToastNotification>,
    pub copy_feedback: Option<CopyFeedback>,
    /// Last reported focus state for the outer terminal hosting herdr.
    /// None means unsupported or not yet reported, which preserves active-pane suppression.
    pub outer_terminal_focus: Option<bool>,
    // Config
    pub prefix_code: KeyCode,
    pub prefix_mods: KeyModifiers,
    pub mobile_width_threshold: u16,
    /// Capture mouse input for Herdr's own mouse UI. When false, Herdr only
    /// captures mouse while the focused pane app requests mouse reporting.
    pub mouse_capture: bool,
    pub right_click_passthrough_modifiers: Option<KeyModifiers>,
    pub right_click_passthrough: Option<RightClickPassthroughGesture>,
    pub redraw_on_focus_gained: bool,
    /// Drop the focused pane cursor while the host window is unfocused, so a
    /// glance at another screen never shows a live-looking caret.
    pub hide_cursor_when_unfocused: bool,
    pub mouse_scroll_lines: usize,
    pub confirm_close: bool,
    pub nerd_font: bool,
    pub show_agent_labels_on_pane_borders: bool,
    pub pane_header: PaneHeaderConfig,
    pub pane_history_persistence: bool,
    /// Refresh Summary asks a headless `grok -p` session for a 5–8 word
    /// headline. Off, it reads the latest prompt from the session log.
    pub refresh_summary_with_grok: bool,
    /// Prompt sent to that headless session. Empty uses the built-in default.
    pub refresh_summary_prompt: String,
    /// Persist the prompt field after it loses focus in Settings.
    pub request_save_refresh_summary_prompt: bool,
    /// Expose the focused pane's cursor anchor to the outer terminal even when
    /// the pane requested `?25l`. See `[experimental] reveal_hidden_cursor_for_cjk_ime`.
    pub reveal_hidden_cursor_for_cjk_ime: bool,
    /// Restrict cursor reveal to focused panes whose detected agent matches
    /// one of these. When false, apply to any focused pane.
    pub cjk_ime_agent_filter_configured: bool,
    pub cjk_ime_agents: Vec<crate::detect::Agent>,
    /// DECSCUSR shape parameter (1–6) for the IME anchor cursor.
    pub cjk_ime_cursor_shape: u8,
    /// While prefix mode is active, switch the macOS host input source to an
    /// ASCII-capable layout so prefix commands register as ASCII even when a
    /// CJK IME is active. macOS only; a no-op elsewhere. See
    /// `[experimental] switch_ascii_input_source_in_prefix`.
    pub switch_ascii_input_source_in_prefix: bool,
    pub kitty_graphics_enabled: bool,
    pub default_shell: String,
    pub shell_mode: crate::config::ShellModeConfig,
    pub new_terminal_cwd: NewTerminalCwdConfig,
    pub pane_scrollback_limit_bytes: usize,
    #[allow(dead_code)] // kept for backward compat; palette.accent is the source of truth
    pub accent: Color,
    pub sound: SoundConfig,
    pub local_sound_playback: bool,
    /// Sound the settings picker wants auditioned. Drained by the server loop,
    /// which plays it locally or asks the foreground client to.
    pub pending_sound_preview: Option<crate::sound::SoundPreview>,
    /// Notify even for the agent in the tab you are looking at.
    /// See `[ui] notify_active_tab`.
    pub notify_active_tab: bool,
    pub toast_config: ToastConfig,
    pub keybinds: Keybinds,
    /// Frame counter for spinner animations (wraps around).
    pub spinner_tick: u32,
    /// UI color palette — all sidebar/UI colors centralized for theming.
    pub palette: Palette,
    /// Currently applied theme name (for settings UI).
    pub theme_name: String,
    /// Settings panel state.
    pub settings: SettingsState,
    /// Cached integration recommendations for onboarding/settings UI.
    pub integration_recommendations: Vec<crate::integration::IntegrationRecommendation>,
    /// Result messages from the latest integration install action.
    pub integration_install_messages: Vec<String>,
    /// Highlight state for the bottom-right global launcher menu.
    pub global_menu: MenuListState,
    /// Resolved host terminal default colors for theming embedded panes.
    pub host_terminal_theme: TerminalTheme,
    /// Set when a persisted session snapshot would change.
    pub session_dirty: bool,
    /// Terminal runtimes that should be shut down by the app/runtime layer
    /// after state has detached their terminal metadata.
    pub(crate) terminal_runtime_shutdowns: Vec<crate::terminal::TerminalId>,
}

impl AppState {
    pub(crate) fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
    }

    pub(crate) fn remove_alias_shadowed_by_new_pane(&mut self, pane_id: PaneId) {
        self.pane_id_aliases.remove(&pane_id.raw());
    }

    pub fn sound_enabled(&self) -> bool {
        self.sound.enabled
    }

    /// Rows of the settings "done sound" picker. A custom mp3 from
    /// `ui.sound.done_path` or `ui.sound.path` leads the list when set,
    /// because it is what actually plays.
    pub fn done_sound_choices(&self) -> Vec<DoneSoundChoice> {
        let custom = self
            .sound
            .path_for(crate::sound::Sound::Done)
            .map(DoneSoundChoice::CustomFile);
        custom
            .into_iter()
            .chain(
                crate::sound::DONE_SOUNDS
                    .iter()
                    .map(DoneSoundChoice::Builtin),
            )
            .collect()
    }

    /// Index into [`AppState::done_sound_choices`] of the sound in use.
    pub fn selected_done_sound_index(&self) -> usize {
        if self.sound.done_sound_overridden_by_path() {
            return 0;
        }
        let key = self.sound.done_sound().key;
        crate::sound::DONE_SOUNDS
            .iter()
            .position(|sound| sound.key == key)
            .unwrap_or(0)
    }

    pub fn toast_delivery(&self) -> ToastDelivery {
        self.toast_config.delivery
    }

    pub fn pane_header(&self) -> PaneHeaderConfig {
        self.pane_header
    }

    pub fn pane_history_persistence_enabled(&self) -> bool {
        self.pane_history_persistence
    }

    pub fn refresh_summary_with_grok(&self) -> bool {
        self.refresh_summary_with_grok
    }

    pub fn refresh_summary_prompt(&self) -> String {
        let trimmed = self.refresh_summary_prompt.trim();
        if trimmed.is_empty() {
            crate::config::DEFAULT_REFRESH_SUMMARY_PROMPT.to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn switch_ascii_input_source_in_prefix_enabled(&self) -> bool {
        self.switch_ascii_input_source_in_prefix
    }

    pub(crate) fn integration_updates_available(&self) -> bool {
        self.integration_recommendations
            .iter()
            .any(|item| item.state == crate::integration::IntegrationStatusKind::Outdated)
    }

    pub(crate) fn global_menu_attention_badge_visible(&self) -> bool {
        self.update_available.is_some() || self.integration_updates_available()
    }

    pub(crate) fn global_menu_item_has_badge(&self, item: &str) -> bool {
        item == "settings" && self.integration_updates_available()
    }

    pub(crate) fn settings_section_has_badge(&self, section: SettingsSection) -> bool {
        section == SettingsSection::Integrations && self.integration_updates_available()
    }

    pub(crate) fn focused_pane_requests_mouse_capture_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mode == Mode::Terminal
            && self
                .terminal_input_target(terminal_runtimes)
                .map(|(_, _, runtime)| runtime)
                .and_then(crate::terminal::TerminalRuntime::input_state)
                .is_some_and(crate::pane::InputState::mouse_reporting_enabled)
    }

    pub(crate) fn should_capture_host_mouse_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mouse_capture || self.focused_pane_requests_mouse_capture_from(terminal_runtimes)
    }

    pub fn is_prefix_key(&self, key: crate::input::TerminalKey) -> bool {
        crate::config::terminal_key_matches_combo(key, (self.prefix_code, self.prefix_mods))
    }

    pub(crate) fn detach_confirm_armed(&self, now: std::time::Instant) -> bool {
        self.detach_confirm_until.is_some_and(|until| now <= until)
    }

    pub(crate) fn arm_detach_confirm(&mut self, now: std::time::Instant) {
        self.detach_confirm_until = Some(now + DETACH_CONFIRM_WINDOW);
    }

    pub(crate) fn clear_detach_confirm(&mut self) {
        self.detach_confirm_until = None;
    }

    pub(crate) fn confirm_detach_if_armed(&mut self, now: std::time::Instant) -> bool {
        if self.detach_confirm_armed(now) {
            self.clear_detach_confirm();
            true
        } else {
            false
        }
    }

    pub fn estimate_pane_size(&self) -> (u16, u16) {
        if let Some(info) = self.view.pane_infos.first() {
            (info.rect.height, info.rect.width)
        } else {
            (24, 80)
        }
    }

    /// Returns true when the given (workspace, tab, pane) refers to the
    /// currently focused pane in the active workspace's active tab.
    pub(crate) fn runtime_for_pane_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        #[cfg(test)]
        if let Some(runtime) = self.workspaces.get(ws_idx)?.test_runtimes.get(&pane_id) {
            return Some(runtime);
        }
        #[cfg(test)]
        if let Some(runtime) = self
            .workspaces
            .get(ws_idx)?
            .tabs
            .iter()
            .find_map(|tab| tab.runtimes.get(&pane_id))
        {
            return Some(runtime);
        }
        let terminal_id = self.workspaces.get(ws_idx)?.terminal_id(pane_id)?;
        terminal_runtimes.get(terminal_id)
    }

    #[cfg(test)]
    pub(crate) fn runtime_for_pane<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        self.workspaces.iter().find_map(|ws| {
            #[cfg(test)]
            if let Some(runtime) = ws.test_runtimes.get(&pane_id) {
                return Some(runtime);
            }
            #[cfg(test)]
            if let Some(runtime) = ws.tabs.iter().find_map(|tab| tab.runtimes.get(&pane_id)) {
                return Some(runtime);
            }
            let terminal_id = ws.terminal_id(pane_id)?;
            terminal_runtimes.get(terminal_id)
        })
    }

    pub(crate) fn focused_runtime_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        let ws = self.workspaces.get(ws_idx)?;
        let pane_id = ws.focused_pane_id()?;
        self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
    }

    pub(crate) fn terminal_input_target<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
    ) -> Option<(
        usize,
        crate::layout::PaneId,
        &'a crate::terminal::TerminalRuntime,
    )> {
        if let Some(pane_id) = self.agent_peek {
            let ws_idx = self.active.unwrap_or(0);
            let runtime = self.runtime_for_agent_pane(terminal_runtimes, pane_id)?;
            Some((ws_idx, pane_id, runtime))
        } else {
            let ws_idx = self.active?;
            let pane_id = self.workspaces.get(ws_idx)?.focused_pane_id()?;
            let runtime = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)?;
            Some((ws_idx, pane_id, runtime))
        }
    }

    pub fn is_active_pane(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> bool {
        let Some(active_ws_idx) = self.active else {
            return false;
        };
        if ws_idx != active_ws_idx {
            return false;
        }
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return false;
        };
        if tab_idx != ws.active_tab_index() {
            return false;
        }
        ws.active_tab().map(|tab| tab.layout.focused()) == Some(pane_id)
    }
}

#[cfg(test)]
pub fn key_matches(
    key: &crossterm::event::KeyEvent,
    expected_code: KeyCode,
    expected_mods: KeyModifiers,
) -> bool {
    crate::config::terminal_key_matches_combo(
        crate::input::TerminalKey::from(*key),
        (expected_code, expected_mods),
    )
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
impl AppState {
    /// Create an AppState for testing — no channels, no PTYs.
    pub fn test_new() -> Self {
        Self {
            terminals: std::collections::HashMap::new(),
            detached_agents: Vec::new(),
            detached_git_statuses: std::collections::HashMap::new(),
            direct_attach_resize_locks: std::collections::HashSet::new(),
            pane_id_aliases: std::collections::HashMap::new(),
            workspaces: Vec::new(),
            active: None,
            previous_pane_focus: None,
            selected: 0,
            mode: Mode::Navigate,
            should_quit: false,
            detach_exits: false,
            detach_requested: false,
            detach_confirm_until: None,
            request_new_workspace: false,
            request_new_tab: false,
            request_new_linked_worktree: None,
            request_open_existing_worktree: None,
            request_new_workspace_cwd: None,
            request_remove_linked_worktree: None,
            request_remove_agent_worktree: None,
            request_submit_worktree_create: false,
            request_submit_worktree_open: false,
            request_submit_worktree_remove: false,
            request_reload_config: false,
            request_client_config_reload: false,
            request_clipboard_write: None,
            creating_new_tab: false,
            requested_new_tab_name: None,
            rename_pane_target: None,
            worktree_create: None,
            worktree_open: None,
            worktree_remove: None,
            worktree_land: None,
            worktree_directory: "/tmp/herdr-worktrees".into(),
            worktree_verify_command: Vec::new(),
            worktree_auto_land: false,
            request_land_worktree: None,
            request_land_agent_prompt: None,
            request_refresh_summary: None,
            landing_worktrees: std::collections::HashSet::new(),
            landing_failures: std::collections::HashMap::new(),
            request_complete_onboarding: false,
            name_input: String::new(),
            name_input_replace_on_type: false,
            composer: crate::composer::ComposerState::default(),
            release_notes: None,
            product_announcement: None,
            keybind_help: KeybindHelpState { scroll: 0 },
            navigator: NavigatorState::default(),
            copy_mode: None,
            agent_order: Vec::new(),
            collapsed_space_keys: std::collections::HashSet::new(),
            collapsed_agent_space_ids: std::collections::HashSet::new(),
            workspace_scroll: 0,
            default_sidebar_width: 26,
            sidebar_width: 26,
            sidebar_min_width: 18,
            sidebar_max_width: 36,
            sidebar_width_source: SidebarWidthSource::ConfigDefault,
            sidebar_width_auto: false,
            sidebar_collapsed: false,
            spaces_collapsed: false,
            sidebar_section_split: 0.5,
            agent_panel_scope: AgentPanelScope::AllWorkspaces,
            agent_table_scroll: 0,
            mobile_switcher_scroll: 0,
            view: ViewState {
                layout: ViewLayout::Desktop,
                composer: crate::ui::ComposerLayout::default(),
                sidebar_rect: Rect::default(),
                workspace_card_areas: Vec::new(),
                agent_row_areas: Vec::new(),
                agent_folder_areas: Vec::new(),
                agent_table: crate::ui::AgentTableLayout::default(),
                agent_locations: std::collections::HashMap::new(),
                terminal_area: Rect::default(),
                mobile_header_rect: Rect::default(),
                mobile_menu_hit_area: Rect::default(),
                toast_hit_area: Rect::default(),
                pane_infos: Vec::new(),
                pane_chrome_controls: Vec::new(),
                pane_title_hit_areas: Vec::new(),
                split_borders: Vec::new(),
            },
            drag: None,
            workspace_press: None,
            pane_press: None,
            agent_press: None,
            sidebar_agent_press: None,
            agent_folder_press: None,
            agent_table_focus: None,
            agent_peek: None,
            confirm_close_agent: None,
            selection: None,
            selection_autoscroll: None,
            context_menu: None,
            update_available: None,
            update_install_command: "herdr update".into(),
            latest_release_notes_available: false,
            update_dismissed: false,
            config_diagnostic: None,
            toast: None,
            copy_feedback: None,
            outer_terminal_focus: None,
            prefix_code: KeyCode::Char('b'),
            prefix_mods: KeyModifiers::CONTROL,
            mobile_width_threshold: crate::config::DEFAULT_MOBILE_WIDTH_THRESHOLD,
            mouse_capture: true,
            right_click_passthrough_modifiers: None,
            right_click_passthrough: None,
            redraw_on_focus_gained: true,
            hide_cursor_when_unfocused: true,
            mouse_scroll_lines: crate::config::DEFAULT_MOUSE_SCROLL_LINES,
            confirm_close: true,
            nerd_font: false,
            show_agent_labels_on_pane_borders: true,
            pane_header: PaneHeaderConfig::default(),
            pane_history_persistence: false,
            refresh_summary_with_grok: false,
            refresh_summary_prompt: crate::config::DEFAULT_REFRESH_SUMMARY_PROMPT.into(),
            request_save_refresh_summary_prompt: false,
            reveal_hidden_cursor_for_cjk_ime: false,
            cjk_ime_agent_filter_configured: false,
            cjk_ime_agents: Vec::new(),
            cjk_ime_cursor_shape: 2, // steady_block
            switch_ascii_input_source_in_prefix: false,
            kitty_graphics_enabled: false,
            default_shell: String::new(),
            shell_mode: crate::config::ShellModeConfig::Auto,
            new_terminal_cwd: NewTerminalCwdConfig::Follow,
            pane_scrollback_limit_bytes: crate::config::DEFAULT_SCROLLBACK_LIMIT_BYTES,
            accent: Color::Cyan,
            sound: SoundConfig {
                enabled: false,
                ..SoundConfig::default()
            },
            local_sound_playback: false,
            pending_sound_preview: None,
            notify_active_tab: false,
            toast_config: ToastConfig::default(),
            keybinds: Keybinds::default(),
            spinner_tick: 0,
            palette: Palette::catppuccin(),
            theme_name: "catppuccin".to_string(),
            settings: SettingsState {
                section: SettingsSection::Theme,
                list: SelectionListState::new(0),
                original_palette: None,
                original_theme: None,
                editing_refresh_prompt: false,
            },
            integration_recommendations: Vec::new(),
            integration_install_messages: Vec::new(),
            global_menu: MenuListState::new(0),
            host_terminal_theme: TerminalTheme::default(),
            session_dirty: false,
            terminal_runtime_shutdowns: Vec::new(),
        }
    }

    /// Populate missing `TerminalState` entries for every pane so tests that
    /// read or write terminal metadata don't need to manually create them.
    pub fn ensure_test_terminals(&mut self) {
        use crate::terminal::TerminalState;
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane in tab.panes.values() {
                    if !self.terminals.contains_key(&pane.attached_terminal_id) {
                        let cwd = ws.identity_cwd.clone();
                        self.terminals.insert(
                            pane.attached_terminal_id.clone(),
                            TerminalState::new(pane.attached_terminal_id.clone(), cwd),
                        );
                    }
                }
            }
        }
    }

    pub fn insert_test_runtime(
        &mut self,
        pane_id: crate::layout::PaneId,
        runtime: crate::terminal::TerminalRuntime,
    ) {
        if let Some(ws) = self
            .workspaces
            .iter_mut()
            .find(|ws| ws.terminal_id(pane_id).is_some())
        {
            ws.insert_test_runtime(pane_id, runtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn built_in_theme_names_resolve() {
        for name in THEME_NAMES {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn light_theme_aliases_resolve() {
        for name in ["light", "latte", "tokyo-day", "onelight", "lotus", "dawn"] {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn key_matches_requires_exact_modifiers() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));

        assert!(!key_matches(
            &KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));
    }

    #[test]
    fn key_matches_letters_case_insensitively() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT),
            KeyCode::Char('b'),
            KeyModifiers::SHIFT,
        ));
    }

    fn agent_menu(space: SpaceMenuKind) -> ContextMenuState {
        ContextMenuState {
            kind: ContextMenuKind::Agent {
                ws_idx: 0,
                pane_id: crate::layout::PaneId::alloc(),
                space,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        }
    }

    fn worktree_delete_idx(menu: &ContextMenuState) -> usize {
        menu.items()
            .iter()
            .position(|item| item == "Delete agent + worktree")
            .expect("delete worktree item")
    }

    #[test]
    fn linked_agent_menu_offers_land_and_destructive_delete() {
        let menu = agent_menu(SpaceMenuKind::LinkedWorktree {
            parent_branch: Some("main".into()),
            already_landed: false,
            in_worktree_directory: true,
        });
        assert_eq!(
            menu.items(),
            vec![
                "Rename agent",
                "Refresh Summary",
                "Delete agent",
                "Land on main",
                "Delete agent + worktree",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        assert!(menu.item_enabled(worktree_delete_idx(&menu)));
    }

    #[test]
    fn repo_agent_menu_keeps_worktree_delete_visible_but_disabled() {
        let menu = agent_menu(SpaceMenuKind::Repo);
        let items = menu.items();
        assert_eq!(
            items,
            vec![
                "Rename agent",
                "Refresh Summary",
                "Delete agent",
                "New worktree",
                "Open worktree...",
                "Delete agent + worktree",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        assert!(!items.iter().any(|item| item.starts_with("Land on")));
        assert!(!menu.item_enabled(worktree_delete_idx(&menu)));
    }

    #[test]
    fn plain_agent_menu_keeps_worktree_delete_visible_but_disabled() {
        let menu = agent_menu(SpaceMenuKind::Plain);
        assert_eq!(
            menu.items(),
            vec![
                "Rename agent",
                "Refresh Summary",
                "Delete agent",
                "Delete agent + worktree",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        assert!(!menu.item_enabled(worktree_delete_idx(&menu)));
    }

    #[test]
    fn linked_agent_menu_grays_worktree_delete_when_directory_is_not_a_worktree() {
        let menu = agent_menu(SpaceMenuKind::LinkedWorktree {
            parent_branch: Some("main".into()),
            already_landed: false,
            in_worktree_directory: false,
        });
        let items = menu.items();
        assert_eq!(items[items.len() - 1], "Delete agent + worktree");
        assert!(!menu.item_enabled(worktree_delete_idx(&menu)));
        let land_idx = items
            .iter()
            .position(|item| is_land_menu_item(item))
            .expect("land item");
        assert!(!menu.item_enabled(land_idx));
    }

    #[test]
    fn landed_worktree_menu_keeps_land_visible_but_disabled() {
        let menu = agent_menu(SpaceMenuKind::LinkedWorktree {
            parent_branch: Some("main".into()),
            already_landed: true,
            in_worktree_directory: true,
        });
        let items = menu.items();
        let land_idx = items
            .iter()
            .position(|item| is_land_menu_item(item))
            .expect("land item");
        assert_eq!(items[land_idx], "Land on main");
        assert!(!menu.item_enabled(land_idx));
        let delete_idx = items
            .iter()
            .position(|item| item == "Delete agent + worktree")
            .expect("delete worktree item");
        assert!(menu.item_enabled(delete_idx));
    }

    #[test]
    fn land_menu_label_uses_the_parent_checkout_branch() {
        assert_eq!(land_menu_label(Some("main")), "Land on main");
        assert_eq!(land_menu_label(Some("release")), "Land on release");
        assert_eq!(land_menu_label(None), "Land on parent");
        assert_eq!(land_menu_label(Some("")), "Land on parent");
        assert!(is_land_menu_item("Land on release"));
        assert!(is_land_menu_item("Land on parent"));
        assert!(!is_land_menu_item("Delete agent + worktree"));
    }

    #[test]
    fn land_prompt_tells_the_agent_to_land_the_clean_branch() {
        assert_eq!(
            land_prompt_text(),
            "Land this clean branch by running `herdr agent worktree land`"
        );
    }
    #[test]
    fn pane_context_menu_offers_close_agent_only_for_agent_panes() {
        let menu_for = |has_agent| ContextMenuState {
            kind: ContextMenuKind::Pane {
                pane_id: crate::layout::PaneId::alloc(),
                has_manual_label: false,
                dimmed: false,
                has_agent,
                can_reset: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };

        assert_eq!(
            menu_for(true).items(),
            [
                "Rename pane",
                "Split vertically",
                "Split horizontally",
                "Zoom",
                "Dim",
                "Close pane",
                "Close agent",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        assert!(!menu_for(false)
            .items()
            .iter()
            .any(|item| item == "Close agent"));
    }
}
