use std::{
    cell::Cell,
    io,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use eframe::egui;
use overcrow_config::{
    CommittedSettingsSaveError, WARFRAME_STEAM_APP_ID, WarframePrefs, WarframePrefsStore,
    WidgetProfile,
};
use overcrow_logging::{Component, LoggerRuntime};
use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};

use super::{
    HttpError, InvasionMission, MarketBackend, MarketClient, MarketCommand, MarketItemDetail,
    MarketItemSummary, MarketSnapshot, WarframeDerivedCache, WorldstateClient, WorldstateSnapshot,
    controller::{
        WARFRAME_PREFS_ERROR_MAX_CHARS, WarframeActionBatch, WarframeController,
        WarframePrefsCommitFailure, WarframePrefsCommitOutcome, bounded_market_query,
        bounded_warframe_prefs_error, commit_warframe_prefs, commit_warframe_prefs_if_dirty,
        drain_ready_warframe_providers, log_warframe_prefs_outcome,
        prune_activity_done_for_snapshot, warframe_actions_allowed,
    },
};
use crate::runtime::{OverlayScheduler, ProviderReadiness, VersionedValue};
use crate::widgets::WidgetManager;

fn snapshot(mode: OverlayMode, steam_app_id: u32) -> CoreSnapshot {
    CoreSnapshot {
        overlay_mode: mode,
        active_game: Some(GameWindow {
            pid: Some(7),
            steam_app_id: Some(steam_app_id),
            app_id: None,
            title: "game".to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
            scale: 1.0,
            backend: "test".to_owned(),
        }),
        ..CoreSnapshot::default()
    }
}

fn test_controller() -> (tempfile::TempDir, WarframeController) {
    let directory = tempfile::tempdir().unwrap();
    let controller = WarframeController::with_dependencies(
        WorldstateClient::with_fetcher(|| Err("offline".to_owned()), || {}),
        MarketClient::default(),
        WarframePrefsStore::from_path(directory.path().join("warframe.json")),
        WarframePrefs::default(),
        ProviderReadiness::default(),
        overcrow_logging::EventLogger::disabled(),
    );
    (directory, controller)
}

#[test]
fn warframe_diagnostic_preferences_use_stable_private_categories() {
    let temp = tempfile::tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let logger = log_runtime.logger();

    log_warframe_prefs_outcome(
        &logger,
        &WarframePrefsCommitOutcome::CommittedWithWarning {
            message: "private durability detail".to_owned(),
        },
    );
    log_warframe_prefs_outcome(
        &logger,
        &WarframePrefsCommitOutcome::RolledBack {
            message: "private filesystem detail".to_owned(),
            category: WarframePrefsCommitFailure::Filesystem,
        },
    );
    drop(logger);
    drop(log_runtime);

    let contents =
        std::fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
    assert!(contents.contains(
        "widget_settings_save_failed affected_widgets=warframe_status,warframe_fissures,warframe_market,warframe_sortie,warframe_invasions category=durability"
    ));
    assert!(contents.contains(
        "widget_settings_save_failed affected_widgets=warframe_status,warframe_fissures,warframe_market,warframe_sortie,warframe_invasions category=filesystem"
    ));
    assert!(!contents.contains("private durability detail"));
    assert!(!contents.contains("private filesystem detail"));
}

#[test]
fn controller_exposes_the_orchestration_interface() {
    let _new: fn(&egui::Context, overcrow_logging::EventLogger) -> WarframeController =
        WarframeController::new;
    let _sync: fn(
        &mut WarframeController,
        &egui::Context,
        &CoreSnapshot,
        &WidgetProfile,
        Instant,
        u64,
    ) = WarframeController::sync;
    let _render: fn(
        &mut WarframeController,
        &mut egui::Ui,
        &mut WidgetManager,
        &CoreSnapshot,
        &mut WidgetProfile,
        f32,
    ) -> bool = WarframeController::render;
}

#[test]
fn provider_gates_follow_game_mode_and_widget_ownership() {
    let (_directory, mut controller) = test_controller();
    let context = egui::Context::default();
    let now = Instant::now();
    let mut profile = WidgetProfile::default();
    profile.warframe_status.enabled = true;
    profile.warframe_market.enabled = true;

    controller.sync(
        &context,
        &snapshot(OverlayMode::Interactive, 620),
        &profile,
        now,
        1_000,
    );
    assert!(!controller.worldstate_enabled);
    assert!(!controller.market_enabled);

    profile.warframe_status.enabled = false;
    controller.sync(
        &context,
        &snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID),
        &profile,
        now,
        1_000,
    );
    assert!(!controller.worldstate_enabled);
    assert!(controller.market_enabled);
}

#[test]
fn passive_disables_market_requests_without_discarding_display_data() {
    let (_directory, mut controller) = test_controller();
    controller.worldstate_snapshot = Arc::new(WorldstateSnapshot {
        error: Some("retained display".to_owned()),
        ..WorldstateSnapshot::default()
    });
    let retained = Arc::clone(&controller.worldstate_snapshot);
    let mut profile = WidgetProfile::default();
    profile.warframe_status.enabled = true;
    profile.warframe_market.enabled = true;

    controller.sync(
        &egui::Context::default(),
        &snapshot(OverlayMode::Passive, WARFRAME_STEAM_APP_ID),
        &profile,
        Instant::now(),
        1_000,
    );

    assert!(controller.worldstate_enabled);
    assert!(!controller.market_enabled);
    assert_eq!(controller.worldstate_snapshot.as_ref(), retained.as_ref());
}

#[test]
fn unauthorized_actions_do_not_save_and_one_authorized_batch_saves_once() {
    let (_directory, mut controller) = test_controller();
    let saves = Cell::new(0);
    let action = || WarframeActionBatch {
        market_query: Some("  arcane energize  ".to_owned()),
        ..WarframeActionBatch::default()
    };

    controller.apply_action_batch_with_save(
        &snapshot(OverlayMode::Interactive, 620),
        action(),
        |_| {
            saves.set(saves.get() + 1);
            Ok(())
        },
    );
    assert_eq!(saves.get(), 0);
    assert!(controller.prefs.last_market_query.is_empty());
    assert_eq!(controller.prefs_revision, 0);

    controller.apply_action_batch_with_save(
        &snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID),
        action(),
        |_| {
            saves.set(saves.get() + 1);
            Ok(())
        },
    );
    assert_eq!(saves.get(), 1);
    assert_eq!(controller.prefs.last_market_query, "arcane energize");
    assert_eq!(controller.prefs_revision, 1);
}

#[test]
fn rollback_preserves_preferences_revision_and_derived_cache() {
    let (_directory, mut controller) = test_controller();
    let context = egui::Context::default();
    let now = Instant::now();
    let mut profile = WidgetProfile::default();
    profile.warframe_status.enabled = true;
    let snapshot = snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID);
    controller.sync(&context, &snapshot, &profile, now, 1_000);
    let previous_prefs = controller.prefs.clone();
    let previous_revision = controller.prefs_revision;
    let previous_counts = controller.derived.recomputation_counts();

    controller.apply_action_batch_with_save(
        &snapshot,
        WarframeActionBatch {
            market_query: Some("failed change".to_owned()),
            ..WarframeActionBatch::default()
        },
        |_| Err(io::Error::other("rename failed")),
    );
    controller.sync(
        &context,
        &snapshot,
        &profile,
        now + Duration::from_millis(10),
        1_000,
    );

    assert_eq!(controller.prefs, previous_prefs);
    assert_eq!(controller.prefs_revision, previous_revision);
    assert_eq!(controller.derived.recomputation_counts(), previous_counts);
}

#[test]
fn market_query_and_copy_flash_deadlines_are_bounded() {
    assert_eq!(
        bounded_market_query(&format!("  {}  ", "é".repeat(80)))
            .chars()
            .count(),
        64
    );

    let (_directory, mut controller) = test_controller();
    let now = Instant::now();
    controller.set_copy_flash("trade-row".to_owned(), now);
    let (_, deadline) = controller.market_copy_flash.as_ref().unwrap();
    assert!(*deadline > now);
    assert!(*deadline <= now + Duration::from_millis(1_400));
    assert!(
        controller
            .scheduler
            .next_repaint_after(now)
            .is_some_and(|delay| delay <= Duration::from_millis(1_400))
    );

    controller.sync(
        &egui::Context::default(),
        &snapshot(OverlayMode::Interactive, 620),
        &WidgetProfile::default(),
        now + Duration::from_millis(1_401),
        1_001,
    );
    assert!(controller.market_copy_flash.is_none());
}

#[test]
fn stopwatch_frames_do_not_repeat_warframe_work() {
    let origin = Instant::now();
    let snapshot = Arc::new(WorldstateSnapshot::default());
    let mut retained_snapshot = Arc::new(WorldstateSnapshot::default());
    let mut retained_revision = 0;
    let mut prefs = WarframePrefs::default();
    let mut prefs_revision = 0;
    let mut cache = WarframeDerivedCache::default();
    let mut scheduler = OverlayScheduler::default();
    let readiness = ProviderReadiness::default();
    let worldstate_reads = Cell::new(0);
    let market_reads = Cell::new(0);
    let preference_clones = Cell::new(0);
    let market_refresh_sends = Cell::new(0);

    let mut first_frame_counts = None;
    for frame in 0..100 {
        if frame == 0 {
            readiness.mark_worldstate();
            readiness.mark_market();
        }
        let updates = drain_ready_warframe_providers(
            &readiness,
            || {
                worldstate_reads.set(worldstate_reads.get() + 1);
                Some(VersionedValue {
                    revision: 7,
                    value: Arc::clone(&snapshot),
                })
            },
            || {
                market_reads.set(market_reads.get() + 1);
                Some(VersionedValue {
                    revision: 11,
                    value: Arc::new(MarketSnapshot::default()),
                })
            },
        );
        if let Some(worldstate) = updates.worldstate {
            retained_revision = worldstate.revision;
            retained_snapshot = worldstate.value;
        }

        cache.sync(
            &retained_snapshot,
            retained_revision,
            &prefs,
            prefs_revision,
        );
        let outcome = commit_warframe_prefs_if_dirty(
            &mut prefs,
            &mut prefs_revision,
            false,
            |current| {
                preference_clones.set(preference_clones.get() + 1);
                current.clone()
            },
            |_| {},
            |_| Ok(()),
        );
        assert!(outcome.is_none());

        let now = origin + Duration::from_millis(frame * 10);
        if scheduler.take_market_refresh(true, true, 11, 1_000, now, 1_000) {
            market_refresh_sends.set(market_refresh_sends.get() + 1);
        }
        let _ = scheduler.take_warframe_tick(true, now);

        let counts = cache.recomputation_counts();
        if frame == 0 {
            first_frame_counts = Some(counts);
        } else {
            assert_eq!(counts, first_frame_counts.unwrap());
            assert_eq!(worldstate_reads.get(), 1);
            assert_eq!(market_reads.get(), 1);
            assert_eq!(preference_clones.get(), 0);
            assert_eq!(market_refresh_sends.get(), 1);
        }
    }

    let counts = first_frame_counts.unwrap();
    assert_eq!(counts.reward_catalog, 1);
    assert_eq!(counts.fissure_indices, 1);
    assert_eq!(counts.invasion_indices, 1);
}

#[test]
fn warframe_actions_require_an_interactive_active_warframe() {
    let mut warframe = snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID);
    assert!(warframe_actions_allowed(&warframe));

    warframe.overlay_mode = OverlayMode::Passive;
    assert!(!warframe_actions_allowed(&warframe));

    let other_game = snapshot(OverlayMode::Interactive, 620);
    assert!(!warframe_actions_allowed(&other_game));

    let no_game = CoreSnapshot {
        overlay_mode: OverlayMode::Interactive,
        ..CoreSnapshot::default()
    };
    assert!(!warframe_actions_allowed(&no_game));
}

#[test]
fn snapshot_pruning_prepares_a_full_candidate_for_current_completion_action() {
    let snapshot = WorldstateSnapshot {
        invasions: vec![InvasionMission {
            instance_id: "current-invasion".to_owned(),
            node: "SolNode1".to_owned(),
            attacker_faction: "Grineer".to_owned(),
            defender_faction: "Corpus".to_owned(),
            attacker_reward: None,
            defender_reward: None,
            count: 0,
            goal: 1,
            completed: false,
        }],
        ..WorldstateSnapshot::default()
    };
    let mut prefs = WarframePrefs {
        activity_done: (0..128)
            .map(|index| format!("invasion:expired-{index}"))
            .collect(),
        ..WarframePrefs::default()
    };

    prune_activity_done_for_snapshot(&mut prefs, &snapshot);
    prefs.toggle_activity_done("invasion:current-invasion");

    assert_eq!(prefs.activity_done, vec!["invasion:current-invasion"]);
}

#[test]
fn invalid_warframe_candidate_does_not_replace_live_prefs_or_call_saver() {
    let mut current = WarframePrefs::default();
    let previous = current.clone();
    let candidate = WarframePrefs {
        show_normal: false,
        show_steel_path: false,
        show_railjack: false,
        ..WarframePrefs::default()
    };
    let save_called = Cell::new(false);

    let result = commit_warframe_prefs(&mut current, candidate, |_| {
        save_called.set(true);
        Ok(())
    });

    assert!(matches!(
        result,
        WarframePrefsCommitOutcome::RolledBack { message, .. }
            if message.contains("at least one fissure source")
                && message.chars().count() <= WARFRAME_PREFS_ERROR_MAX_CHARS
    ));
    assert!(!save_called.get());
    assert_eq!(current, previous);
}

#[test]
fn failed_warframe_save_does_not_replace_live_prefs() {
    let mut current = WarframePrefs::default();
    let previous = current.clone();
    let candidate = WarframePrefs {
        show_normal: false,
        ..WarframePrefs::default()
    };
    let save_called = Cell::new(false);
    let oversized_detail = "é".repeat(WARFRAME_PREFS_ERROR_MAX_CHARS * 2);

    let result = commit_warframe_prefs(&mut current, candidate, |_| {
        save_called.set(true);
        Err(io::Error::other(oversized_detail))
    });

    assert!(matches!(
        result,
        WarframePrefsCommitOutcome::RolledBack { message, .. }
            if message.contains("Could not apply Warframe preferences")
                && message.chars().count() <= WARFRAME_PREFS_ERROR_MAX_CHARS
    ));
    assert!(save_called.get());
    assert_eq!(current, previous);
}

#[test]
fn committed_warframe_save_publishes_candidate_with_bounded_durability_warning() {
    let mut current = WarframePrefs::default();
    let candidate = WarframePrefs {
        show_normal: false,
        activity_done: vec!["sortie:1:0".into(), "sortie:1:0".into()],
        ..WarframePrefs::default()
    };
    let expected = candidate.clone().validate().unwrap();
    let oversized_detail = "é".repeat(WARFRAME_PREFS_ERROR_MAX_CHARS * 2);

    let outcome = commit_warframe_prefs(&mut current, candidate, |_| {
        Err(io::Error::other(CommittedSettingsSaveError::new(
            io::Error::other(oversized_detail),
        )))
    });

    assert_eq!(current, expected);
    assert!(matches!(
        outcome,
        WarframePrefsCommitOutcome::CommittedWithWarning { message }
            if message.contains("durability")
                && message.chars().count() <= WARFRAME_PREFS_ERROR_MAX_CHARS
    ));
}

#[test]
fn preference_revision_advances_only_for_published_candidates() {
    let mut prefs = WarframePrefs::default();
    let mut revision = 3;

    let durable = commit_warframe_prefs_if_dirty(
        &mut prefs,
        &mut revision,
        true,
        WarframePrefs::clone,
        |candidate| candidate.show_normal = false,
        |_| Ok(()),
    );
    assert!(matches!(durable, Some(WarframePrefsCommitOutcome::Durable)));
    assert_eq!(revision, 4);

    let warning = commit_warframe_prefs_if_dirty(
        &mut prefs,
        &mut revision,
        true,
        WarframePrefs::clone,
        |candidate| candidate.show_normal = true,
        |_| {
            Err(io::Error::other(CommittedSettingsSaveError::new(
                io::Error::other("directory sync failed"),
            )))
        },
    );
    assert!(matches!(
        warning,
        Some(WarframePrefsCommitOutcome::CommittedWithWarning { .. })
    ));
    assert_eq!(revision, 5);

    let rollback = commit_warframe_prefs_if_dirty(
        &mut prefs,
        &mut revision,
        true,
        WarframePrefs::clone,
        |candidate| candidate.show_normal = false,
        |_| Err(io::Error::other("rename failed")),
    );
    assert!(matches!(
        rollback,
        Some(WarframePrefsCommitOutcome::RolledBack { .. })
    ));
    assert_eq!(revision, 5);
    assert!(prefs.show_normal);
}

#[test]
fn warframe_preference_error_is_bounded_for_nonmodal_display() {
    let message = bounded_warframe_prefs_error("é".repeat(WARFRAME_PREFS_ERROR_MAX_CHARS * 2));

    assert!(message.starts_with("Could not apply Warframe preferences"));
    assert!(message.chars().count() <= WARFRAME_PREFS_ERROR_MAX_CHARS);
}

struct ControllerGateBarrierBackend {
    order_started: mpsc::Sender<()>,
    release_order: mpsc::Receiver<()>,
    search_started: mpsc::Sender<()>,
    release_search: mpsc::Receiver<()>,
}

impl MarketBackend for ControllerGateBarrierBackend {
    fn search(&mut self, _query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        self.search_started.send(()).unwrap();
        self.release_search.recv().unwrap();
        Ok(Vec::new())
    }

    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError> {
        self.order_started.send(()).unwrap();
        self.release_order.recv().unwrap();
        Ok(MarketItemDetail {
            name: "Test Item".to_owned(),
            slug: slug.to_owned(),
            lowest_sell: Some(42),
            highest_buy: None,
            order_count: 0,
            top_sells: Vec::new(),
            top_buys: Vec::new(),
        })
    }
}

fn assert_controller_gate_suppresses_inflight_result(
    mutate: impl FnOnce(&mut CoreSnapshot, &mut WidgetProfile),
) {
    let (order_started_tx, order_started_rx) = mpsc::channel();
    let (release_order_tx, release_order_rx) = mpsc::channel();
    let (search_started_tx, search_started_rx) = mpsc::channel();
    let (release_search_tx, release_search_rx) = mpsc::channel();
    let market = MarketClient::with_backend(ControllerGateBarrierBackend {
        order_started: order_started_tx,
        release_order: release_order_rx,
        search_started: search_started_tx,
        release_search: release_search_rx,
    });
    let directory = tempfile::tempdir().unwrap();
    let mut controller = WarframeController::with_dependencies(
        WorldstateClient::with_fetcher(|| Err("offline".to_owned()), || {}),
        market,
        WarframePrefsStore::from_path(directory.path().join("warframe.json")),
        WarframePrefs::default(),
        ProviderReadiness::default(),
        overcrow_logging::EventLogger::disabled(),
    );
    let context = egui::Context::default();
    let now = Instant::now();
    let mut core = snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID);
    let mut profile = WidgetProfile::default();
    profile.warframe_market.enabled = true;
    controller.sync(&context, &core, &profile, now, 1_000);
    controller
        .market_client
        .send(MarketCommand::Select("test_item".to_owned()));
    order_started_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    mutate(&mut core, &mut profile);
    controller.sync(&context, &core, &profile, now, 1_000);
    release_order_tx.send(()).unwrap();

    core = snapshot(OverlayMode::Interactive, WARFRAME_STEAM_APP_ID);
    profile.warframe_market.enabled = true;
    controller.sync(&context, &core, &profile, now, 1_000);
    controller
        .market_client
        .send(MarketCommand::Search("barrier".to_owned()));
    search_started_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let selected = controller.market_client.latest().selected;
    release_search_tx.send(()).unwrap();
    drop(controller);

    assert!(selected.is_none());
}

#[test]
fn passive_request_immediately_invalidates_an_inflight_market_generation() {
    assert_controller_gate_suppresses_inflight_result(|snapshot, _profile| {
        snapshot.overlay_mode = OverlayMode::Passive;
    });
}

#[test]
fn catalog_disable_immediately_invalidates_an_inflight_market_generation() {
    assert_controller_gate_suppresses_inflight_result(|_snapshot, profile| {
        profile.warframe_market.enabled = false;
    });
}
