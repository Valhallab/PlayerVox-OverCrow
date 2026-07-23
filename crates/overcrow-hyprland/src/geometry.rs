use overcrow_protocol::{CoreSnapshot, OverlayMode, Rect};

use crate::model::{HyprWindow, OVERLAY_APP_ID, WindowAddress};

#[derive(Clone, Debug, PartialEq)]
struct LiveClient {
    address: WindowAddress,
    rect: Rect,
    workspace_id: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct TrackedIdentity {
    pid: u32,
    app_id: String,
    address: WindowAddress,
}

#[derive(Debug, Default)]
pub struct GeometrySynchronizer {
    identity: Option<TrackedIdentity>,
    converged: Option<Rect>,
    last_dispatch: Option<Vec<String>>,
}

impl GeometrySynchronizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn commands(
        &mut self,
        snapshot: &CoreSnapshot,
        active: Option<&HyprWindow>,
        clients: &[HyprWindow],
    ) -> Vec<String> {
        let Some(game) = snapshot.active_game.as_ref() else {
            return self.reset();
        };
        let (Some(pid), Some(app_id)) = (game.pid, game.app_id.as_deref()) else {
            return self.reset();
        };
        let pid_i64 = i64::from(pid);
        let matching_games = clients
            .iter()
            .filter(|window| {
                window.mapped
                    && !window.hidden
                    && window.pid == pid_i64
                    && window.class == app_id
                    && window.class != OVERLAY_APP_ID
            })
            .collect::<Vec<_>>();
        let [game_window] = matching_games.as_slice() else {
            return self.reset();
        };
        let Some(live_game) = live_client(game_window) else {
            return self.reset();
        };

        let identity = TrackedIdentity {
            pid,
            app_id: app_id.to_owned(),
            address: live_game.address.clone(),
        };
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.converged = None;
            self.last_dispatch = None;
        }

        let overlays = clients
            .iter()
            .filter(|window| window.mapped && !window.hidden && window.class == OVERLAY_APP_ID)
            .filter_map(live_client)
            .collect::<Vec<_>>();
        if overlays.is_empty() {
            self.converged = None;
            self.last_dispatch = None;
            return Vec::new();
        }

        let overlays_match_game = overlays.iter().all(|overlay| {
            overlay.rect == live_game.rect && overlay.workspace_id == live_game.workspace_id
        });
        if overlays_match_game {
            self.converged = Some(live_game.rect);
            self.last_dispatch = None;
            return Vec::new();
        }

        if snapshot.overlay_mode == OverlayMode::Interactive
            && self.converged.as_ref() == Some(&live_game.rect)
            && let Some(active_overlay) = active_overlay(active, &overlays)
            && active_overlay.workspace_id == live_game.workspace_id
            && active_overlay.rect != live_game.rect
            && overlays.iter().all(|overlay| {
                overlay.address == active_overlay.address
                    || (overlay.rect == live_game.rect
                        && overlay.workspace_id == live_game.workspace_id)
            })
        {
            return self.dispatch_once(exact_geometry_commands(
                &live_game.address,
                &active_overlay.rect,
            ));
        }

        let commands = overlays
            .iter()
            .filter(|overlay| {
                overlay.rect != live_game.rect || overlay.workspace_id != live_game.workspace_id
            })
            .flat_map(|overlay| overlay_follow_commands(overlay, &live_game))
            .collect();
        self.dispatch_once(commands)
    }

    fn reset(&mut self) -> Vec<String> {
        self.identity = None;
        self.converged = None;
        self.last_dispatch = None;
        Vec::new()
    }

    fn dispatch_once(&mut self, commands: Vec<String>) -> Vec<String> {
        if commands.is_empty() || self.last_dispatch.as_ref() == Some(&commands) {
            return Vec::new();
        }
        self.last_dispatch = Some(commands.clone());
        commands
    }
}

fn live_client(window: &HyprWindow) -> Option<LiveClient> {
    if !window.mapped || window.hidden {
        return None;
    }
    let workspace_id = window.workspace.as_ref()?.id;
    if workspace_id <= 0 {
        return None;
    }
    Some(LiveClient {
        address: WindowAddress::parse(&window.address)?,
        rect: window.rect()?,
        workspace_id,
    })
}

fn active_overlay<'a>(
    active: Option<&HyprWindow>,
    overlays: &'a [LiveClient],
) -> Option<&'a LiveClient> {
    let active = active?;
    if active.class != OVERLAY_APP_ID {
        return None;
    }
    let address = WindowAddress::parse(&active.address)?;
    overlays.iter().find(|overlay| overlay.address == address)
}

fn exact_geometry_commands(address: &WindowAddress, rect: &Rect) -> Vec<String> {
    vec![
        format!(
            "dispatch resizewindowpixel exact {} {},address:{}",
            rect.width,
            rect.height,
            address.as_str()
        ),
        format!(
            "dispatch movewindowpixel exact {} {},address:{}",
            rect.x,
            rect.y,
            address.as_str()
        ),
    ]
}

fn overlay_follow_commands(overlay: &LiveClient, game: &LiveClient) -> Vec<String> {
    let mut commands = vec![format!(
        "dispatch movetoworkspacesilent {},address:{}",
        game.workspace_id,
        overlay.address.as_str()
    )];
    commands.extend(exact_geometry_commands(&overlay.address, &game.rect));
    commands.push(format!(
        "dispatch alterzorder top,address:{}",
        overlay.address.as_str()
    ));
    commands
}

#[cfg(test)]
mod tests {
    use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};

    use crate::model::{HyprWindow, HyprWorkspace, OVERLAY_APP_ID};

    use super::GeometrySynchronizer;

    const GAME_APP_ID: &str = "steam_app_1623730";

    fn sample_window(address: &str, class: &str, pid: i64) -> HyprWindow {
        HyprWindow {
            address: address.to_owned(),
            mapped: true,
            hidden: false,
            at: [12, 36],
            size: [2417, 1680],
            monitor: 0,
            class: class.to_owned(),
            title: class.to_owned(),
            pid,
            workspace: Some(HyprWorkspace {
                id: 1,
                name: "1".to_owned(),
            }),
            tags: Vec::new(),
        }
    }

    fn snapshot(mode: OverlayMode) -> CoreSnapshot {
        CoreSnapshot {
            active_game: Some(GameWindow {
                pid: Some(136_279),
                steam_app_id: Some(1_623_730),
                app_id: Some(GAME_APP_ID.to_owned()),
                title: "Pal".to_owned(),
                rect: Rect {
                    x: 12,
                    y: 36,
                    width: 2417,
                    height: 1680,
                },
                scale: 1.25,
                backend: "wayland".to_owned(),
            }),
            overlay_mode: mode,
            session_elapsed_ms: None,
            ..CoreSnapshot::default()
        }
    }

    fn fixture() -> (CoreSnapshot, HyprWindow, HyprWindow) {
        (
            snapshot(OverlayMode::Interactive),
            sample_window("0x10", GAME_APP_ID, 136_279),
            sample_window("0x20", OVERLAY_APP_ID, 42),
        )
    }

    fn converge(
        synchronizer: &mut GeometrySynchronizer,
        snapshot: &CoreSnapshot,
        game: &HyprWindow,
        overlay: &HyprWindow,
    ) {
        assert!(
            synchronizer
                .commands(snapshot, Some(game), &[game.clone(), overlay.clone()],)
                .is_empty()
        );
    }

    fn overlay_commands(x: i32, y: i32, width: u32, height: u32) -> Vec<String> {
        vec![
            "dispatch movetoworkspacesilent 1,address:0x20".to_owned(),
            format!("dispatch resizewindowpixel exact {width} {height},address:0x20"),
            format!("dispatch movewindowpixel exact {x} {y},address:0x20"),
            "dispatch alterzorder top,address:0x20".to_owned(),
        ]
    }

    fn game_commands(x: i32, y: i32, width: u32, height: u32) -> Vec<String> {
        vec![
            format!("dispatch resizewindowpixel exact {width} {height},address:0x10"),
            format!("dispatch movewindowpixel exact {x} {y},address:0x10"),
        ]
    }

    #[test]
    fn initial_mismatch_aligns_overlay_to_game_without_resizing_game() {
        let (snapshot, game, mut overlay) = fixture();
        overlay.at = [200, 300];

        assert_eq!(
            GeometrySynchronizer::new().commands(&snapshot, Some(&game), &[game.clone(), overlay],),
            overlay_commands(12, 36, 2417, 1680)
        );
    }

    #[test]
    fn active_overlay_only_delta_is_proxied_to_the_validated_game() {
        let (snapshot, game, mut overlay) = fixture();
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &snapshot, &game, &overlay);
        overlay.at = [-88, 36];
        overlay.size = [2517, 1680];

        assert_eq!(
            synchronizer.commands(&snapshot, Some(&overlay), &[game, overlay.clone()]),
            game_commands(-88, 36, 2517, 1680)
        );
    }

    #[test]
    fn unchanged_mismatch_is_dispatched_only_once() {
        let (snapshot, game, mut overlay) = fixture();
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &snapshot, &game, &overlay);
        overlay.at = [-88, 36];
        overlay.size = [2517, 1680];

        assert_eq!(
            synchronizer.commands(&snapshot, Some(&overlay), &[game.clone(), overlay.clone()],),
            game_commands(-88, 36, 2517, 1680)
        );
        assert!(
            synchronizer
                .commands(&snapshot, Some(&overlay), &[game, overlay.clone()])
                .is_empty()
        );
    }

    #[test]
    fn passive_overlay_delta_is_not_proxied_to_the_game() {
        let (_, game, mut overlay) = fixture();
        let passive = snapshot(OverlayMode::Passive);
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &passive, &game, &overlay);
        overlay.at = [-88, 36];

        assert_eq!(
            synchronizer.commands(&passive, Some(&overlay), &[game, overlay.clone()]),
            overlay_commands(12, 36, 2417, 1680)
        );
    }

    #[test]
    fn game_delta_is_authoritative_and_realigns_the_overlay() {
        let (snapshot, mut game, overlay) = fixture();
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &snapshot, &game, &overlay);
        game.at = [112, 36];

        assert_eq!(
            synchronizer.commands(&snapshot, Some(&game), &[game.clone(), overlay]),
            overlay_commands(112, 36, 2417, 1680)
        );
    }

    #[test]
    fn simultaneous_game_and_overlay_delta_prefers_the_game() {
        let (snapshot, mut game, mut overlay) = fixture();
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &snapshot, &game, &overlay);
        game.at = [112, 36];
        overlay.at = [-88, 36];

        assert_eq!(
            synchronizer.commands(&snapshot, Some(&overlay), &[game, overlay.clone()]),
            overlay_commands(112, 36, 2417, 1680)
        );
    }

    #[test]
    fn third_party_focus_never_makes_the_overlay_authoritative() {
        let (snapshot, game, mut overlay) = fixture();
        let code = sample_window("0x30", "code", 30_331);
        let mut synchronizer = GeometrySynchronizer::new();
        converge(&mut synchronizer, &snapshot, &game, &overlay);
        overlay.at = [-88, 36];

        assert_eq!(
            synchronizer.commands(&snapshot, Some(&code), &[game, overlay, code.clone()],),
            overlay_commands(12, 36, 2417, 1680)
        );
    }

    #[test]
    fn ambiguous_or_mismatched_game_identity_emits_no_commands() {
        let (snapshot, game, overlay) = fixture();
        let mut duplicate = game.clone();
        duplicate.address = "0x11".to_owned();
        let mut synchronizer = GeometrySynchronizer::new();

        assert!(
            synchronizer
                .commands(
                    &snapshot,
                    Some(&game),
                    &[game.clone(), duplicate, overlay.clone()],
                )
                .is_empty()
        );

        let mut wrong_app = snapshot;
        wrong_app.active_game.as_mut().expect("active game").app_id = Some("wrong".to_owned());
        assert!(
            synchronizer
                .commands(&wrong_app, Some(&game), &[game.clone(), overlay])
                .is_empty()
        );
    }
}
