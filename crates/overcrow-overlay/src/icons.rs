use eframe::egui::{Align2, Color32, FontDefinitions, FontId, Painter, Rect};
use egui_phosphor::regular;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppIcon {
    Add,
    Check,
    CheckCircle,
    Circle,
    Clock,
    Close,
    Headphones,
    Discord,
    Fissures,
    Invasions,
    Market,
    MediaNext,
    MediaPause,
    MediaPlay,
    MediaPrevious,
    MicrophoneMuted,
    Missions,
    Notes,
    Options,
    PassiveHidden,
    PassiveVisible,
    Performance,
    Reply,
    Session,
    Star,
    Stopwatch,
    Twitch,
    WarframeStatus,
}

impl AppIcon {
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Add => regular::PLUS,
            Self::Check => regular::CHECK,
            Self::CheckCircle => regular::CHECK_CIRCLE,
            Self::Circle => regular::CIRCLE,
            Self::Clock => regular::CLOCK,
            Self::Close => regular::X,
            Self::Headphones => regular::HEADPHONES,
            Self::Discord => regular::DISCORD_LOGO,
            Self::Fissures => regular::ATOM,
            Self::Invasions => regular::SWORD,
            Self::Market => regular::STOREFRONT,
            Self::MediaNext => regular::SKIP_FORWARD,
            Self::MediaPause => regular::PAUSE,
            Self::MediaPlay => regular::PLAY,
            Self::MediaPrevious => regular::SKIP_BACK,
            Self::MicrophoneMuted => regular::MICROPHONE_SLASH,
            Self::Missions => regular::TARGET,
            Self::Notes => regular::NOTE,
            Self::Options => regular::DOTS_THREE_VERTICAL,
            Self::PassiveHidden => regular::EYE_SLASH,
            Self::PassiveVisible => regular::EYE,
            Self::Performance => regular::CHART_BAR,
            Self::Reply => regular::ARROW_BEND_UP_LEFT,
            Self::Session => regular::HOURGLASS,
            Self::Star => regular::STAR,
            Self::Stopwatch => regular::TIMER,
            Self::Twitch => regular::TWITCH_LOGO,
            Self::WarframeStatus => regular::PLANET,
        }
    }
}

pub fn add_to_fonts(fonts: &mut FontDefinitions) {
    egui_phosphor::add_to_fonts(fonts, egui_phosphor::Variant::Regular);
}

pub fn paint_icon_at(painter: &Painter, rect: Rect, icon: AppIcon, color: Color32) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon.glyph(),
        FontId::proportional(rect.height()),
        color,
    );
}

#[cfg(test)]
mod tests {
    use eframe::egui::{FontDefinitions, FontFamily};

    use super::{AppIcon, add_to_fonts};

    #[test]
    fn semantic_icons_use_the_expected_phosphor_glyphs() {
        assert_eq!(
            AppIcon::Discord.glyph(),
            egui_phosphor::regular::DISCORD_LOGO
        );
        assert_eq!(AppIcon::Twitch.glyph(), egui_phosphor::regular::TWITCH_LOGO);
        assert_eq!(
            AppIcon::MicrophoneMuted.glyph(),
            egui_phosphor::regular::MICROPHONE_SLASH
        );
        assert_eq!(
            AppIcon::Headphones.glyph(),
            egui_phosphor::regular::HEADPHONES
        );
        assert_eq!(AppIcon::PassiveVisible.glyph(), egui_phosphor::regular::EYE);
        assert_eq!(
            AppIcon::PassiveHidden.glyph(),
            egui_phosphor::regular::EYE_SLASH
        );
        assert_eq!(
            AppIcon::Options.glyph(),
            egui_phosphor::regular::DOTS_THREE_VERTICAL
        );
        assert_eq!(AppIcon::Close.glyph(), egui_phosphor::regular::X);
        assert_eq!(AppIcon::Star.glyph(), egui_phosphor::regular::STAR);
        assert_eq!(AppIcon::MediaPlay.glyph(), egui_phosphor::regular::PLAY);
        assert_eq!(AppIcon::MediaPause.glyph(), egui_phosphor::regular::PAUSE);
        assert_eq!(
            AppIcon::Reply.glyph(),
            egui_phosphor::regular::ARROW_BEND_UP_LEFT
        );
    }

    #[test]
    fn phosphor_font_is_registered_as_a_proportional_fallback() {
        let mut fonts = FontDefinitions::default();

        add_to_fonts(&mut fonts);

        assert!(fonts.font_data.contains_key("phosphor"));
        assert!(
            fonts.families[&FontFamily::Proportional]
                .iter()
                .any(|font| font == "phosphor")
        );
    }
}
