use overcrow_config::WidgetId;

use crate::icons::AppIcon;

#[cfg(test)]
mod tests {
    use overcrow_config::WidgetId;

    use super::{BUILTIN_WIDGETS, WidgetCategory};

    #[test]
    fn registry_groups_every_widget_once_in_stable_product_order() {
        assert_eq!(
            WidgetCategory::ALL,
            [WidgetCategory::General, WidgetCategory::Warframe]
        );

        let general = BUILTIN_WIDGETS
            .iter()
            .filter(|widget| widget.category == WidgetCategory::General)
            .map(|widget| widget.id)
            .collect::<Vec<_>>();
        assert_eq!(
            general,
            [
                WidgetId::Session,
                WidgetId::Clock,
                WidgetId::Performance,
                WidgetId::ManualStopwatch,
                WidgetId::Media,
                WidgetId::Notes,
                WidgetId::TwitchChat,
                WidgetId::DiscordVoice,
            ]
        );

        let warframe = BUILTIN_WIDGETS
            .iter()
            .filter(|widget| widget.category == WidgetCategory::Warframe)
            .map(|widget| widget.id)
            .collect::<Vec<_>>();
        assert_eq!(
            warframe,
            [
                WidgetId::WarframeStatus,
                WidgetId::WarframeFissures,
                WidgetId::WarframeMarket,
                WidgetId::WarframeSortie,
                WidgetId::WarframeInvasions,
            ]
        );

        for id in WidgetId::ALL {
            assert_eq!(
                BUILTIN_WIDGETS
                    .iter()
                    .filter(|widget| widget.id == id)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn category_copy_explains_game_scope_without_runtime_detection() {
        assert_eq!(WidgetCategory::General.label(), "General");
        assert_eq!(
            WidgetCategory::General.description(),
            "Useful in every supported game"
        );
        assert_eq!(WidgetCategory::Warframe.label(), "Warframe");
        assert_eq!(
            WidgetCategory::Warframe.description(),
            "Live public data and local Warframe tools"
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WidgetDescriptor {
    pub id: WidgetId,
    pub name: &'static str,
    pub description: &'static str,
    pub category: WidgetCategory,
    pub glyph: WidgetGlyph,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetCategory {
    General,
    Warframe,
}

impl WidgetCategory {
    pub const ALL: [Self; 2] = [Self::General, Self::Warframe];

    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Warframe => "Warframe",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::General => "Useful in every supported game",
            Self::Warframe => "Live public data and local Warframe tools",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetGlyph {
    Session,
    Clock,
    Performance,
    Stopwatch,
    Media,
    Notes,
    Twitch,
    Discord,
    Warframe,
    Fissures,
    Market,
    Missions,
    Invasions,
}

impl WidgetGlyph {
    pub(super) const fn app_icon(self) -> AppIcon {
        match self {
            Self::Session => AppIcon::Session,
            Self::Clock => AppIcon::Clock,
            Self::Performance => AppIcon::Performance,
            Self::Stopwatch => AppIcon::Stopwatch,
            Self::Media => AppIcon::MediaPlay,
            Self::Notes => AppIcon::Notes,
            Self::Twitch => AppIcon::Twitch,
            Self::Discord => AppIcon::Discord,
            Self::Warframe => AppIcon::WarframeStatus,
            Self::Fissures => AppIcon::Fissures,
            Self::Market => AppIcon::Market,
            Self::Missions => AppIcon::Missions,
            Self::Invasions => AppIcon::Invasions,
        }
    }
}

pub const BUILTIN_WIDGETS: [WidgetDescriptor; WidgetId::COUNT] = [
    WidgetDescriptor {
        id: WidgetId::Session,
        name: "Session",
        description: "Elapsed time since the game session started.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Session,
    },
    WidgetDescriptor {
        id: WidgetId::Clock,
        name: "Clock",
        description: "Local time and date.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Clock,
    },
    WidgetDescriptor {
        id: WidgetId::Performance,
        name: "Performance",
        description: "Host CPU, memory, and temperatures.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Performance,
    },
    WidgetDescriptor {
        id: WidgetId::ManualStopwatch,
        name: "Manual stopwatch",
        description: "In-game stopwatch with controls and shortcuts.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Stopwatch,
    },
    WidgetDescriptor {
        id: WidgetId::Media,
        name: "Media",
        description: "Active MPRIS media and playback controls.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Media,
    },
    WidgetDescriptor {
        id: WidgetId::Notes,
        name: "Notes",
        description: "Local note and checklist.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Notes,
    },
    WidgetDescriptor {
        id: WidgetId::TwitchChat,
        name: "Twitch chat",
        description: "Read and send messages in a selected public Twitch chat.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Twitch,
    },
    WidgetDescriptor {
        id: WidgetId::DiscordVoice,
        name: "Discord voice",
        description: "See who is present and speaking in Discord voice.",
        category: WidgetCategory::General,
        glyph: WidgetGlyph::Discord,
    },
    WidgetDescriptor {
        id: WidgetId::WarframeStatus,
        name: "Warframe status",
        description: "Open-world cycles, daily reset, and Baro (public data).",
        category: WidgetCategory::Warframe,
        glyph: WidgetGlyph::Warframe,
    },
    WidgetDescriptor {
        id: WidgetId::WarframeFissures,
        name: "Fissures",
        description: "Active void fissures with local filters.",
        category: WidgetCategory::Warframe,
        glyph: WidgetGlyph::Fissures,
    },
    WidgetDescriptor {
        id: WidgetId::WarframeMarket,
        name: "Market",
        description: "warframe.market search and trade templates.",
        category: WidgetCategory::Warframe,
        glyph: WidgetGlyph::Market,
    },
    WidgetDescriptor {
        id: WidgetId::WarframeSortie,
        name: "Sortie & Archon",
        description: "Daily Sortie and Archon Hunt (public data).",
        category: WidgetCategory::Warframe,
        glyph: WidgetGlyph::Missions,
    },
    WidgetDescriptor {
        id: WidgetId::WarframeInvasions,
        name: "Invasions",
        description: "Active invasions, progress, and rewards.",
        category: WidgetCategory::Warframe,
        glyph: WidgetGlyph::Invasions,
    },
];
