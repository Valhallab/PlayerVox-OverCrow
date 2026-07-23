use overcrow_config::{WARFRAME_STEAM_APP_ID, WidgetId, WidgetProfile};
use overcrow_protocol::{CoreSnapshot, OverlayMode};

pub fn is_warframe_active(snapshot: &CoreSnapshot) -> bool {
    snapshot
        .active_game
        .as_ref()
        .and_then(|game| game.steam_app_id)
        .is_some_and(|id| id == WARFRAME_STEAM_APP_ID)
}

pub fn any_worldstate_widget_enabled(profile: &WidgetProfile) -> bool {
    [
        WidgetId::WarframeStatus,
        WidgetId::WarframeFissures,
        WidgetId::WarframeSortie,
        WidgetId::WarframeInvasions,
    ]
    .into_iter()
    .any(|id| profile.settings(id).enabled)
}

pub fn market_requests_enabled(snapshot: &CoreSnapshot, profile: &WidgetProfile) -> bool {
    is_warframe_active(snapshot)
        && snapshot.overlay_mode == OverlayMode::Interactive
        && profile.settings(WidgetId::WarframeMarket).enabled
}

pub fn warframe_widget_visible(
    id: WidgetId,
    snapshot: &CoreSnapshot,
    _profile: &WidgetProfile,
    base_visible: bool,
) -> bool {
    matches!(
        id,
        WidgetId::WarframeStatus
            | WidgetId::WarframeFissures
            | WidgetId::WarframeMarket
            | WidgetId::WarframeSortie
            | WidgetId::WarframeInvasions
    ) && base_visible
        && is_warframe_active(snapshot)
}

#[cfg(test)]
mod tests {
    use overcrow_config::{WidgetId, WidgetProfile};
    use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};

    use super::{
        any_worldstate_widget_enabled, is_warframe_active, market_requests_enabled,
        warframe_widget_visible,
    };

    fn game(steam_app_id: Option<u32>) -> CoreSnapshot {
        CoreSnapshot {
            active_game: Some(GameWindow {
                pid: Some(1),
                steam_app_id,
                app_id: None,
                title: "game".to_owned(),
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                scale: 1.0,
                backend: "test".to_owned(),
            }),
            ..CoreSnapshot::default()
        }
    }

    #[test]
    fn gate_requires_warframe_steam_id() {
        assert!(is_warframe_active(&game(Some(230_410))));
        assert!(!is_warframe_active(&game(Some(620))));
        assert!(!is_warframe_active(&game(None)));
        assert!(!is_warframe_active(&CoreSnapshot::default()));
    }

    #[test]
    fn warframe_visibility_requires_gate_and_base_policy() {
        let mut profile = WidgetProfile::default();
        profile.warframe_status.enabled = true;
        let warframe = game(Some(230_410));
        let other = game(Some(620));
        assert!(warframe_widget_visible(
            WidgetId::WarframeStatus,
            &warframe,
            &profile,
            true
        ));
        assert!(!warframe_widget_visible(
            WidgetId::WarframeStatus,
            &other,
            &profile,
            true
        ));
        assert!(!warframe_widget_visible(
            WidgetId::WarframeStatus,
            &warframe,
            &profile,
            false
        ));
    }

    #[test]
    fn worldstate_polling_excludes_a_market_only_profile() {
        let mut profile = WidgetProfile::default();
        profile.warframe_market.enabled = true;
        assert!(!any_worldstate_widget_enabled(&profile));
        profile.warframe_fissures.enabled = true;
        assert!(any_worldstate_widget_enabled(&profile));
    }

    #[test]
    fn market_requests_require_interactive_warframe_and_enabled_widget() {
        let mut profile = WidgetProfile::default();
        let mut warframe = game(Some(230_410));
        warframe.overlay_mode = OverlayMode::Interactive;

        assert!(!market_requests_enabled(&warframe, &profile));
        profile.warframe_market.enabled = true;
        assert!(market_requests_enabled(&warframe, &profile));

        warframe.overlay_mode = OverlayMode::Passive;
        assert!(!market_requests_enabled(&warframe, &profile));

        let mut other_game = game(Some(620));
        other_game.overlay_mode = OverlayMode::Interactive;
        assert!(!market_requests_enabled(&other_game, &profile));
    }
}
