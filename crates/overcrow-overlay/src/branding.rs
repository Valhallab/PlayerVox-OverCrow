//! PlayerVox brand block for overlay chrome.
//!
//! Mirrors the PlayerVox frontend `Logo` component (lime accent icon box +
//! uppercase italic black wordmark with accent on `VOX`), and adds an OverCrow
//! product subtitle under the wordmark so the overlay reads as a PlayerVox
//! product.

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Pos2, Sense, Shape,
    Stroke, Vec2, epaint::PathShape, pos2, vec2,
};
use egui::text::{LayoutJob, TextFormat};

/// Default PlayerVox accent (lime), matching `DEFAULT_ACCENT` in PlayerVoxFront.
pub const BRAND_ACCENT: Color32 = Color32::from_rgb(0xa3, 0xe6, 0x35);
const WORDMARK_WHITE: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfa);
const SUBTITLE_MUTED: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xaa); // zinc-400
/// Dark bars on the lime icon box (PlayerVox default / non-hover state).
const MARK_ON_ACCENT: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0a);

pub const BRAND_WORDMARK_FAMILY: &str = "NotoSansBrand";
pub const BRAND_SUBTITLE_FAMILY: &str = "NotoSansBrandSub";

/// Source SVG viewBox aspect (width / height) for the mark paths.
const MARK_ASPECT: f32 = 340.0 / 400.0;

const BRAND_WORDMARK_FONT: &[u8] =
    include_bytes!("../../../assets/branding/NotoSans-BlackItalic-OverCrow.ttf");
const BRAND_SUBTITLE_FONT: &[u8] =
    include_bytes!("../../../assets/branding/NotoSans-Regular-OverCrow.ttf");

// Normalized [0,1] contours from assets/branding/playervox-mark.svg
// (viewBox 85 55 340 400).
const BAR1: &[[f32; 2]] = &[
    [0.09403, 0.29375],
    [0.04806, 0.30680],
    [0.02098, 0.34000],
    [0.01785, 0.38207],
    [0.01785, 0.42385],
    [0.01785, 0.46563],
    [0.01785, 0.50742],
    [0.01785, 0.54920],
    [0.01785, 0.59098],
    [0.01785, 0.63276],
    [0.01785, 0.67454],
    [0.01785, 0.71632],
    [0.01785, 0.75811],
    [0.01785, 0.79989],
    [0.01785, 0.84167],
    [0.01785, 0.88345],
    [0.01785, 0.92523],
    [0.01785, 0.96701],
    [0.04115, 0.96149],
    [0.07617, 0.93218],
    [0.11119, 0.90286],
    [0.14621, 0.87354],
    [0.18123, 0.84422],
    [0.21625, 0.81490],
    [0.25127, 0.78558],
    [0.28629, 0.75626],
    [0.31138, 0.72341],
    [0.31138, 0.68162],
    [0.31138, 0.63984],
    [0.31138, 0.59806],
    [0.31138, 0.55628],
    [0.31138, 0.51450],
    [0.31138, 0.47272],
    [0.31138, 0.43094],
    [0.31138, 0.38915],
    [0.30981, 0.34705],
    [0.28603, 0.31194],
    [0.24179, 0.29574],
    [0.19233, 0.29497],
    [0.14318, 0.29436],
];

const BAR2: &[[f32; 2]] = &[
    [0.43235, 0.15250],
    [0.38372, 0.16747],
    [0.35807, 0.20475],
    [0.35676, 0.24975],
    [0.35676, 0.29442],
    [0.35676, 0.33908],
    [0.35676, 0.38375],
    [0.35676, 0.42842],
    [0.35676, 0.47308],
    [0.35676, 0.51775],
    [0.35676, 0.56242],
    [0.35676, 0.60708],
    [0.35676, 0.65175],
    [0.35676, 0.69642],
    [0.36480, 0.73425],
    [0.41735, 0.73425],
    [0.46990, 0.73425],
    [0.52245, 0.73425],
    [0.57500, 0.73425],
    [0.62755, 0.73425],
    [0.65059, 0.70917],
    [0.65059, 0.66450],
    [0.65059, 0.61983],
    [0.65059, 0.57517],
    [0.65059, 0.53050],
    [0.65059, 0.48583],
    [0.65059, 0.44117],
    [0.65059, 0.39650],
    [0.65059, 0.35183],
    [0.65059, 0.30717],
    [0.65059, 0.26250],
    [0.65059, 0.21783],
    [0.63377, 0.17623],
    [0.59039, 0.15382],
    [0.53745, 0.15250],
    [0.48490, 0.15250],
];

const BAR3: &[[f32; 2]] = &[
    [0.76765, 0.01375],
    [0.71253, 0.03393],
    [0.69206, 0.08189],
    [0.69206, 0.13433],
    [0.69206, 0.18678],
    [0.69206, 0.23922],
    [0.69206, 0.29167],
    [0.69206, 0.34411],
    [0.69206, 0.39656],
    [0.69206, 0.44900],
    [0.69206, 0.50144],
    [0.69206, 0.55389],
    [0.69206, 0.60633],
    [0.69206, 0.65878],
    [0.69206, 0.71122],
    [0.72520, 0.73550],
    [0.78690, 0.73550],
    [0.84859, 0.73550],
    [0.91029, 0.73550],
    [0.97199, 0.73550],
    [0.98588, 0.69486],
    [0.98588, 0.64242],
    [0.98588, 0.58997],
    [0.98588, 0.53753],
    [0.98588, 0.48508],
    [0.98588, 0.43264],
    [0.98588, 0.38019],
    [0.98588, 0.32775],
    [0.98588, 0.27531],
    [0.98588, 0.22286],
    [0.98588, 0.17042],
    [0.98588, 0.11797],
    [0.98438, 0.06513],
    [0.95084, 0.02369],
    [0.89105, 0.01375],
    [0.82935, 0.01375],
];

/// Brand assets holder (kept for call-site stability).
#[derive(Default)]
pub struct BrandAssets {}

/// Install brand faces into the egui font atlas. Call once at startup.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Greyscale AA — RGB subpixel looks speckled on translucent dark chrome.
    let wordmark = FontData::from_static(BRAND_WORDMARK_FONT).tweak(FontTweak {
        hinting: Some(true),
        subpixel_binning: Some(false),
        ..FontTweak::default()
    });
    let subtitle = FontData::from_static(BRAND_SUBTITLE_FONT).tweak(FontTweak {
        hinting: Some(true),
        subpixel_binning: Some(false),
        ..FontTweak::default()
    });
    fonts.font_data.insert(
        BRAND_WORDMARK_FAMILY.to_owned(),
        std::sync::Arc::new(wordmark),
    );
    fonts.font_data.insert(
        BRAND_SUBTITLE_FAMILY.to_owned(),
        std::sync::Arc::new(subtitle),
    );
    fonts.families.insert(
        FontFamily::Name(BRAND_WORDMARK_FAMILY.into()),
        vec![BRAND_WORDMARK_FAMILY.to_owned()],
    );
    fonts.families.insert(
        FontFamily::Name(BRAND_SUBTITLE_FAMILY.into()),
        vec![BRAND_SUBTITLE_FAMILY.to_owned()],
    );
    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrandSize {
    Sm,
    Md,
}

impl BrandSize {
    fn word_size(self) -> f32 {
        match self {
            Self::Sm => 16.0, // text-base
            Self::Md => 20.0, // text-xl-ish
        }
    }

    fn subtitle_size(self) -> f32 {
        match self {
            Self::Sm => 10.0,
            Self::Md => 11.5,
        }
    }

    /// Gap between wordmark and product subtitle.
    fn text_stack_gap(self) -> f32 {
        match self {
            Self::Sm => 1.0,
            Self::Md => 2.0,
        }
    }

    fn gap(self) -> f32 {
        match self {
            Self::Sm => 8.0,
            Self::Md => 10.0,
        }
    }

    /// CSS `tracking-tighter` (~-0.05em).
    fn letter_spacing(self) -> f32 {
        -self.word_size() * 0.05
    }

    /// Corner radius scales with the icon box (≈10/32 of side, like Logo.tsx).
    fn box_radius(self, box_side: f32) -> f32 {
        (box_side * (10.0 / 32.0)).clamp(6.0, 12.0)
    }

    /// Mark height inside the box (~78% of the shorter box edge).
    fn mark_size(self, box_side: f32) -> Vec2 {
        let height = box_side * 0.78;
        vec2(height * MARK_ASPECT, height)
    }
}

fn wordmark_font_id(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BRAND_WORDMARK_FAMILY.into()))
}

fn subtitle_font_id(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BRAND_SUBTITLE_FAMILY.into()))
}

/// Paint PlayerVox logo + OverCrow product subtitle.
///
/// Layout — icon box is a square matching the full text-stack height so
/// PLAYERVOX and OverCrow both align with it:
/// ```text
/// ┌────┐ PLAYERVOX
/// │ ◧  │ OverCrow
/// └────┘
/// ```
pub fn paint_brand(ui: &mut egui::Ui, _assets: &mut BrandAssets, size: BrandSize) {
    let word_size = size.word_size();
    let subtitle_size = size.subtitle_size();
    let gap = size.gap();
    let tracking = size.letter_spacing();
    let stack_gap = size.text_stack_gap();

    let word_job = playervox_wordmark_job(&wordmark_font_id(word_size), tracking);
    let subtitle_job = overcrow_subtitle_job(&subtitle_font_id(subtitle_size));

    // Measure text first so the lime box can match the full stack height.
    let word_galley = ui.fonts_mut(|fonts| fonts.layout_job(word_job));
    let subtitle_galley = ui.fonts_mut(|fonts| fonts.layout_job(subtitle_job));
    let stack_height = word_galley.size().y + stack_gap + subtitle_galley.size().y;

    let ppp = ui.pixels_per_point();
    let box_side = (stack_height * ppp).round() / ppp;
    let mark_size = size.mark_size(box_side);
    let radius = size.box_radius(box_side);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_min_height(box_side);

        paint_icon_box(ui, box_side, mark_size, radius);

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = stack_gap;
            ui.set_min_height(box_side);
            ui.add(egui::Label::new(word_galley).selectable(false));
            ui.add(egui::Label::new(subtitle_galley).selectable(false));
        });
    });
}

fn paint_icon_box(ui: &mut egui::Ui, box_side: f32, mark_size: Vec2, radius: f32) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::splat(box_side), Sense::hover());
    let painter = ui.painter();

    // Accent-filled rounded square sized to the full wordmark+subtitle stack.
    painter.rect_filled(rect, radius, BRAND_ACCENT);

    let mark_rect = egui::Rect::from_center_size(rect.center(), mark_size);
    paint_mark_in(painter, mark_rect, MARK_ON_ACCENT);
}

fn paint_mark_in(painter: &egui::Painter, rect: egui::Rect, fill: Color32) {
    for contour in [BAR1, BAR2, BAR3] {
        let points: Vec<Pos2> = contour
            .iter()
            .map(|&[u, v]| {
                pos2(
                    rect.left() + u * rect.width(),
                    rect.top() + v * rect.height(),
                )
            })
            .collect();
        painter.add(Shape::Path(PathShape {
            points,
            closed: true,
            fill,
            stroke: Stroke::NONE.into(),
        }));
    }
}

fn playervox_wordmark_job(font_id: &FontId, letter_spacing: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    // Web Logo uses mixed-case "Player"+"Vox" with CSS uppercase; we paint the
    // rendered uppercase form directly so the subset stays tiny.
    let white = TextFormat {
        font_id: font_id.clone(),
        color: WORDMARK_WHITE,
        extra_letter_spacing: letter_spacing,
        ..Default::default()
    };
    let accent = TextFormat {
        font_id: font_id.clone(),
        color: BRAND_ACCENT,
        extra_letter_spacing: letter_spacing,
        ..Default::default()
    };
    job.append("PLAYER", 0.0, white);
    job.append("VOX", 0.0, accent);
    job
}

fn overcrow_subtitle_job(font_id: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        "OverCrow",
        0.0,
        TextFormat {
            font_id: font_id.clone(),
            color: SUBTITLE_MUTED,
            ..Default::default()
        },
    );
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_accent_matches_playervox_lime() {
        assert_eq!(BRAND_ACCENT, Color32::from_rgb(163, 230, 53));
    }

    #[test]
    fn mark_contours_are_closed_and_normalized() {
        for contour in [BAR1, BAR2, BAR3] {
            assert!(contour.len() >= 8);
            for &[u, v] in contour {
                assert!((0.0..=1.0).contains(&u), "u={u}");
                assert!((0.0..=1.0).contains(&v), "v={v}");
            }
        }
        let bar1_top = BAR1.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let bar2_top = BAR2.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let bar3_top = BAR3.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        assert!(bar3_top < bar2_top && bar2_top < bar1_top);
    }

    #[test]
    fn embedded_fonts_are_loadable() {
        for bytes in [BRAND_WORDMARK_FONT, BRAND_SUBTITLE_FONT] {
            assert!(bytes.len() > 100);
            assert!(
                bytes.starts_with(&[0x00, 0x01, 0x00, 0x00])
                    || bytes.starts_with(b"OTTO")
                    || bytes.starts_with(b"true")
            );
        }
    }

    #[test]
    fn brand_size_md_is_larger_than_sm() {
        assert!(BrandSize::Md.word_size() > BrandSize::Sm.word_size());
        assert!(BrandSize::Md.subtitle_size() > BrandSize::Sm.subtitle_size());
        let sm_box = BrandSize::Sm.word_size() + BrandSize::Sm.subtitle_size();
        let md_box = BrandSize::Md.word_size() + BrandSize::Md.subtitle_size();
        assert!(md_box > sm_box);
        assert!(BrandSize::Md.mark_size(32.0).y > BrandSize::Sm.mark_size(24.0).y);
    }

    #[test]
    fn icon_box_matches_text_stack_height() {
        // Box side is derived from the stacked wordmark + subtitle, not a fixed
        // web-only icon size — so both lines align with the lime tile.
        let word = BrandSize::Sm.word_size();
        let sub = BrandSize::Sm.subtitle_size();
        let gap = BrandSize::Sm.text_stack_gap();
        let stack = word + gap + sub;
        assert!(stack > word);
        assert!(BrandSize::Sm.box_radius(stack) >= 6.0);
        assert!(BrandSize::Sm.mark_size(stack).y < stack);
    }

    #[test]
    fn playervox_wordmark_splits_player_and_vox() {
        let font = wordmark_font_id(18.0);
        let job = playervox_wordmark_job(&font, -0.9);
        assert_eq!(job.sections.len(), 2);
        assert_eq!(job.text, "PLAYERVOX");
        assert_eq!(job.sections[0].format.color, WORDMARK_WHITE);
        assert_eq!(job.sections[1].format.color, BRAND_ACCENT);
    }

    #[test]
    fn overcrow_subtitle_names_the_product() {
        let font = subtitle_font_id(11.0);
        let job = overcrow_subtitle_job(&font);
        assert_eq!(job.text, "OverCrow");
        assert_eq!(job.sections[0].format.color, SUBTITLE_MUTED);
    }
}
