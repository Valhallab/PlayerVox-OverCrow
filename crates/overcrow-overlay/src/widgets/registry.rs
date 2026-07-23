use overcrow_config::WidgetId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WidgetDescriptor {
    pub id: WidgetId,
    pub name: &'static str,
    pub description: &'static str,
}

pub const BUILTIN_WIDGETS: [WidgetDescriptor; 11] = [
    WidgetDescriptor {
        id: WidgetId::Session,
        name: "Session",
        description: "Elapsed time since the game session started.",
    },
    WidgetDescriptor {
        id: WidgetId::Clock,
        name: "Clock",
        description: "Local time and date.",
    },
    WidgetDescriptor {
        id: WidgetId::Performance,
        name: "Performance",
        description: "Host CPU, memory, and temperatures.",
    },
    WidgetDescriptor {
        id: WidgetId::ManualStopwatch,
        name: "Manual stopwatch",
        description: "In-game stopwatch with controls and shortcuts.",
    },
    WidgetDescriptor {
        id: WidgetId::Media,
        name: "Media",
        description: "Active MPRIS media and playback controls.",
    },
    WidgetDescriptor {
        id: WidgetId::Notes,
        name: "Notes",
        description: "Local note and checklist.",
    },
    WidgetDescriptor {
        id: WidgetId::WarframeStatus,
        name: "Warframe status",
        description: "Open-world cycles, daily reset, and Baro (public data).",
    },
    WidgetDescriptor {
        id: WidgetId::WarframeFissures,
        name: "Fissures",
        description: "Active void fissures with local filters.",
    },
    WidgetDescriptor {
        id: WidgetId::WarframeMarket,
        name: "Market",
        description: "warframe.market search and trade templates.",
    },
    WidgetDescriptor {
        id: WidgetId::WarframeSortie,
        name: "Sortie & Archon",
        description: "Daily Sortie and Archon Hunt (public data).",
    },
    WidgetDescriptor {
        id: WidgetId::WarframeInvasions,
        name: "Invasions",
        description: "Active invasions, progress, and rewards.",
    },
];
