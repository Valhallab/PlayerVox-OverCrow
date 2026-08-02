use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Id, Painter, Rect,
    TextureHandle, TextureOptions, Vec2, pos2,
};

const TABLER_FONT_NAME: &str = "overcrow-tabler-icons";
const TABLER_FONT: &[u8] = include_bytes!("../../../assets/icons/tabler-icons-overcrow.ttf");
const DISCORD_MARK: &[u8] = include_bytes!("../../../assets/icons/discord-symbol-blurple.png");
const TWITCH_MARK: &[u8] = include_bytes!("../../../assets/icons/twitch-glitch-purple.png");

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BrandIcon {
    Discord,
    Twitch,
}

impl BrandIcon {
    const fn bytes(self) -> &'static [u8] {
        match self {
            Self::Discord => DISCORD_MARK,
            Self::Twitch => TWITCH_MARK,
        }
    }

    const fn texture_name(self) -> &'static str {
        match self {
            Self::Discord => "overcrow-discord-mark",
            Self::Twitch => "overcrow-twitch-mark",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IconSource {
    Tabler(&'static str),
    Brand(BrandIcon),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppIcon {
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
    const fn source(self) -> IconSource {
        match self {
            Self::Add => IconSource::Tabler("\u{eb0b}"),
            Self::Check => IconSource::Tabler("\u{ea5e}"),
            Self::CheckCircle => IconSource::Tabler("\u{ea67}"),
            Self::Circle => IconSource::Tabler("\u{ea6b}"),
            Self::Clock => IconSource::Tabler("\u{ea70}"),
            Self::Close => IconSource::Tabler("\u{eb55}"),
            Self::Headphones => IconSource::Tabler("\u{ed1d}"),
            Self::Discord => IconSource::Brand(BrandIcon::Discord),
            Self::Fissures => IconSource::Tabler("\u{eb79}"),
            Self::Invasions => IconSource::Tabler("\u{f030}"),
            Self::Market => IconSource::Tabler("\u{ea4e}"),
            Self::MediaNext => IconSource::Tabler("\u{ed49}"),
            Self::MediaPause => IconSource::Tabler("\u{ed45}"),
            Self::MediaPlay => IconSource::Tabler("\u{ed46}"),
            Self::MediaPrevious => IconSource::Tabler("\u{ed48}"),
            Self::MicrophoneMuted => IconSource::Tabler("\u{ed16}"),
            Self::Missions => IconSource::Tabler("\u{eb35}"),
            Self::Notes => IconSource::Tabler("\u{eb6d}"),
            Self::Options => IconSource::Tabler("\u{ea94}"),
            Self::PassiveHidden => IconSource::Tabler("\u{ecf0}"),
            Self::PassiveVisible => IconSource::Tabler("\u{ea9a}"),
            Self::Performance => IconSource::Tabler("\u{ea59}"),
            Self::Reply => IconSource::Tabler("\u{eb77}"),
            Self::Session => IconSource::Tabler("\u{ef93}"),
            Self::Star => IconSource::Tabler("\u{eb2e}"),
            Self::Stopwatch => IconSource::Tabler("\u{ff9b}"),
            Self::Twitch => IconSource::Brand(BrandIcon::Twitch),
            Self::WarframeStatus => IconSource::Tabler("\u{ec08}"),
        }
    }

    #[cfg(test)]
    pub(crate) const fn glyph(self) -> Option<&'static str> {
        match self.source() {
            IconSource::Tabler(glyph) => Some(glyph),
            IconSource::Brand(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_brand(self) -> bool {
        matches!(self.source(), IconSource::Brand(_))
    }
}

pub(crate) fn add_to_fonts(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        TABLER_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(TABLER_FONT)),
    );
    fonts
        .families
        .insert(tabler_font_family(), vec![TABLER_FONT_NAME.to_owned()]);
}

pub(crate) fn tabler_font_family() -> FontFamily {
    FontFamily::Name(TABLER_FONT_NAME.into())
}

pub(crate) fn paint_icon_at(painter: &Painter, rect: Rect, icon: AppIcon, color: Color32) {
    match icon.source() {
        IconSource::Tabler(glyph) => {
            let family = tabler_font_family();
            let font_ready = painter
                .ctx()
                .fonts(|fonts| fonts.families().contains(&family));
            if !font_ready {
                return;
            }
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::new(rect.height(), family),
                color,
            );
        }
        IconSource::Brand(brand) => paint_brand_icon(painter, rect, brand, color.a()),
    }
}

fn paint_brand_icon(painter: &Painter, rect: Rect, brand: BrandIcon, alpha: u8) {
    let Some(texture) = brand_texture(painter.ctx(), brand) else {
        return;
    };
    let fitted = fit_rect(rect, texture.size_vec2());
    painter.image(
        texture.id(),
        fitted,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE.gamma_multiply(f32::from(alpha) / 255.0),
    );
}

fn brand_texture(ctx: &egui::Context, brand: BrandIcon) -> Option<TextureHandle> {
    let cache_id = Id::new(("overcrow-brand-icon", brand));
    if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(cache_id)) {
        return Some(texture);
    }

    let decoded = image::load_from_memory(brand.bytes()).ok()?.into_rgba8();
    let width = usize::try_from(decoded.width()).ok()?;
    let height = usize::try_from(decoded.height()).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([width, height], decoded.as_raw());
    let texture = ctx.load_texture(brand.texture_name(), image, TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(cache_id, texture.clone()));
    Some(texture)
}

fn fit_rect(target: Rect, source_size: Vec2) -> Rect {
    if source_size.x <= 0.0 || source_size.y <= 0.0 {
        return target;
    }
    let scale = (target.width() / source_size.x).min(target.height() / source_size.y);
    Rect::from_center_size(target.center(), source_size * scale)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{FontDefinitions, FontFamily, Rect, pos2, vec2};

    use super::{AppIcon, DISCORD_MARK, TABLER_FONT_NAME, TWITCH_MARK, add_to_fonts, fit_rect};

    #[test]
    fn semantic_icons_use_the_expected_tabler_glyphs() {
        assert_eq!(AppIcon::MicrophoneMuted.glyph(), Some("\u{ed16}"));
        assert_eq!(AppIcon::Headphones.glyph(), Some("\u{ed1d}"));
        assert_eq!(AppIcon::PassiveVisible.glyph(), Some("\u{ea9a}"));
        assert_eq!(AppIcon::PassiveHidden.glyph(), Some("\u{ecf0}"));
        assert_eq!(AppIcon::Options.glyph(), Some("\u{ea94}"));
        assert_eq!(AppIcon::Close.glyph(), Some("\u{eb55}"));
        assert_eq!(AppIcon::Star.glyph(), Some("\u{eb2e}"));
        assert_eq!(AppIcon::MediaPlay.glyph(), Some("\u{ed46}"));
        assert_eq!(AppIcon::MediaPause.glyph(), Some("\u{ed45}"));
        assert_eq!(AppIcon::Reply.glyph(), Some("\u{eb77}"));
    }

    #[test]
    fn discord_and_twitch_use_embedded_brand_marks() {
        assert!(AppIcon::Discord.is_brand());
        assert!(AppIcon::Twitch.is_brand());
        assert_eq!(AppIcon::Discord.glyph(), None);
        assert_eq!(AppIcon::Twitch.glyph(), None);

        for bytes in [DISCORD_MARK, TWITCH_MARK] {
            let image = image::load_from_memory(bytes).expect("embedded brand mark must decode");
            assert!(image.width() <= 128);
            assert!(image.height() <= 128);
            assert!(image.width() > 0);
            assert!(image.height() > 0);
        }
    }

    #[test]
    fn tabler_font_uses_a_dedicated_family() {
        let mut fonts = FontDefinitions::default();

        add_to_fonts(&mut fonts);

        let family = FontFamily::Name(TABLER_FONT_NAME.into());
        assert!(fonts.font_data.contains_key(TABLER_FONT_NAME));
        assert_eq!(fonts.families[&family], [TABLER_FONT_NAME]);
        assert!(
            !fonts.families[&FontFamily::Proportional]
                .iter()
                .any(|font| font == TABLER_FONT_NAME)
        );
    }

    #[test]
    fn subset_contains_every_mapped_tabler_glyph() {
        let context = eframe::egui::Context::default();
        let mut fonts = FontDefinitions::default();
        add_to_fonts(&mut fonts);
        context.set_fonts(fonts);
        let family = super::tabler_font_family();
        let icons = [
            AppIcon::Add,
            AppIcon::Check,
            AppIcon::CheckCircle,
            AppIcon::Circle,
            AppIcon::Clock,
            AppIcon::Close,
            AppIcon::Headphones,
            AppIcon::Fissures,
            AppIcon::Invasions,
            AppIcon::Market,
            AppIcon::MediaNext,
            AppIcon::MediaPause,
            AppIcon::MediaPlay,
            AppIcon::MediaPrevious,
            AppIcon::MicrophoneMuted,
            AppIcon::Missions,
            AppIcon::Notes,
            AppIcon::Options,
            AppIcon::PassiveHidden,
            AppIcon::PassiveVisible,
            AppIcon::Performance,
            AppIcon::Reply,
            AppIcon::Session,
            AppIcon::Star,
            AppIcon::Stopwatch,
            AppIcon::WarframeStatus,
        ];

        let _ = context.run_ui(eframe::egui::RawInput::default(), |ui| {
            ui.fonts_mut(|fonts| {
                for icon in icons {
                    let glyph = icon.glyph().expect("listed icon must be a font glyph");
                    let character = glyph.chars().next().expect("glyph must not be empty");
                    assert!(
                        fonts
                            .has_glyph(&eframe::egui::FontId::new(16.0, family.clone()), character),
                        "missing glyph for {icon:?}"
                    );
                }
            });
        });
    }

    #[test]
    fn brand_marks_preserve_their_aspect_ratio() {
        let target = Rect::from_min_max(pos2(0.0, 0.0), pos2(40.0, 40.0));
        let wide = fit_rect(target, vec2(128.0, 97.0));
        let tall = fit_rect(target, vec2(110.0, 128.0));

        assert_eq!(wide.width(), 40.0);
        assert!(wide.height() < 40.0);
        assert_eq!(tall.height(), 40.0);
        assert!(tall.width() < 40.0);
        assert_eq!(wide.center(), target.center());
        assert_eq!(tall.center(), target.center());
    }
}
