use overcrow_core::{NoopWindowSource, ProcessClassification, WindowObservation, WindowSource};
use overcrow_protocol::{GameWindow, Rect};

#[test]
fn observation_into_game_copies_metadata_and_injects_steam_id() {
    let observation = WindowObservation {
        pid: Some(42),
        app_id: Some("portal2".to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: 100,
            y: 200,
            width: 1920,
            height: 1080,
        },
        scale: 1.25,
        backend: "x11".to_owned(),
    };
    let classification = ProcessClassification {
        steam_app_id: Some(620),
        is_game_candidate: true,
    };

    let game = observation.into_game(classification);

    assert_eq!(
        game,
        GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 100,
                y: 200,
                width: 1920,
                height: 1080,
            },
            scale: 1.25,
            backend: "x11".to_owned(),
        }
    );
}

#[test]
fn noop_source_reports_no_active_window() {
    let mut source = NoopWindowSource;

    assert_eq!(source.active_window().expect("no-op source succeeds"), None);
}
