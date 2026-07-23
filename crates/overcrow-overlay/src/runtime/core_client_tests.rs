use super::{
    Backoff, BaselineFailure, COMMAND_CHANNEL_CAPACITY, ClientCommand, CommandResponseMode,
    ConnectionEvent, ConnectionEventTracker, ConnectionGenerationDecision, OwnerStreamEvent,
    PassiveIntent, RevisionDecision, RevisionGate, SnapshotClient, SnapshotUpdate,
    VersionedHandling, apply_versioned, baseline_failure_for_error_name, command_channel,
    handle_command_response, handle_signal_json, owner_stream_decision, publish_json,
    snapshot_channel,
};
use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect, VersionedCoreSnapshot};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn interactive_snapshot() -> CoreSnapshot {
    CoreSnapshot {
        active_game: Some(GameWindow {
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
            scale: 1.0,
            backend: "test".to_owned(),
        }),
        overlay_mode: OverlayMode::Interactive,
        session_elapsed_ms: None,
        ..CoreSnapshot::default()
    }
}

fn event(revision: u64, snapshot: CoreSnapshot) -> VersionedCoreSnapshot {
    VersionedCoreSnapshot { revision, snapshot }
}

#[test]
fn connection_events_deduplicate_failures_and_report_recovery() {
    let mut tracker = ConnectionEventTracker::default();

    assert_eq!(
        tracker.failed("bus unavailable"),
        Some(ConnectionEvent::ConnectionFailed(
            "bus unavailable".to_owned()
        ))
    );
    assert_eq!(tracker.failed("bus unavailable"), None);
    assert_eq!(tracker.connected(), Some(ConnectionEvent::Connected));
    assert_eq!(tracker.connected(), None);
    assert_eq!(
        tracker.failed("owner vanished"),
        Some(ConnectionEvent::Disconnected("owner vanished".to_owned()))
    );
    assert_eq!(tracker.failed("owner vanished"), None);
}

#[test]
fn connection_errors_are_bounded_before_they_are_retained() {
    let mut tracker = ConnectionEventTracker::default();
    let event = tracker.failed(format!("private\n{}", "x".repeat(8_192)));
    let rendered = format!("{event:?}");

    assert!(rendered.len() < 512);
}

#[test]
fn revisions_apply_new_ignore_equal_and_reconcile_conflicts() {
    let first = event(4, interactive_snapshot());
    let mut gate = RevisionGate::default();
    assert_eq!(
        gate.apply(first.clone()),
        RevisionDecision::Apply(first.clone())
    );
    assert_eq!(gate.apply(first.clone()), RevisionDecision::Ignore);
    assert_eq!(
        gate.apply(event(3, CoreSnapshot::default())),
        RevisionDecision::Ignore
    );
    assert_eq!(
        gate.apply(event(4, CoreSnapshot::default())),
        RevisionDecision::Reconcile
    );
}

#[tokio::test(start_paused = true)]
async fn command_notification_wakes_without_advancing_tokio_time() {
    let (command_sender, command_receiver) = command_channel();
    let started_at = tokio::time::Instant::now();
    let notified = command_receiver.notified();

    assert!(command_sender.send(ClientCommand::ReloadWidgetSettings));

    tokio::select! {
        () = notified => {}
        () = tokio::time::sleep(Duration::from_secs(1)) => {
            panic!("command notification did not wake the receiver");
        }
    }
    assert_eq!(tokio::time::Instant::now(), started_at);
}

#[test]
fn queued_newer_and_stale_signals_are_ordered_against_the_baseline() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let mut gate = RevisionGate::default();
    let mut intent = PassiveIntent::default();

    assert_eq!(
        apply_versioned(
            event(4, CoreSnapshot::default()),
            &mut gate,
            &sender,
            &mut intent,
            || {
                repaints.fetch_add(1, Ordering::SeqCst);
            },
        ),
        VersionedHandling::Applied
    );
    assert_eq!(
        apply_versioned(
            event(5, interactive_snapshot()),
            &mut gate,
            &sender,
            &mut intent,
            || {
                repaints.fetch_add(1, Ordering::SeqCst);
            },
        ),
        VersionedHandling::Applied
    );
    assert_eq!(
        apply_versioned(
            event(3, CoreSnapshot::default()),
            &mut gate,
            &sender,
            &mut intent,
            || {
                repaints.fetch_add(1, Ordering::SeqCst);
            },
        ),
        VersionedHandling::Ignored
    );

    assert_eq!(
        receiver.take_latest(),
        Some(SnapshotUpdate::confirmed(interactive_snapshot(), true))
    );
    assert_eq!(repaints.load(Ordering::SeqCst), 2);
}

#[test]
fn malformed_signal_keeps_last_good_snapshot_and_requests_reconciliation() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let mut gate = RevisionGate::default();
    let mut intent = PassiveIntent::default();

    assert_eq!(
        apply_versioned(
            event(4, interactive_snapshot()),
            &mut gate,
            &sender,
            &mut intent,
            || {
                repaints.fetch_add(1, Ordering::SeqCst);
            },
        ),
        VersionedHandling::Applied
    );
    assert_eq!(
        handle_signal_json("not json", &mut gate, &sender, &mut intent, || {
            repaints.fetch_add(1, Ordering::SeqCst);
        },),
        VersionedHandling::Reconcile
    );

    assert_eq!(
        receiver.take_latest(),
        Some(SnapshotUpdate::confirmed(interactive_snapshot(), false))
    );
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn revision_gate_resets_for_each_connection_generation() {
    let mut old_generation = RevisionGate::default();
    assert!(matches!(
        old_generation.apply(event(100, interactive_snapshot())),
        RevisionDecision::Apply(_)
    ));

    let mut new_generation = RevisionGate::default();
    assert_eq!(
        new_generation.apply(event(1, CoreSnapshot::default())),
        RevisionDecision::Apply(event(1, CoreSnapshot::default()))
    );
}

#[test]
fn owner_replacement_reconnects_before_low_revision_enters_the_prior_gate() {
    let mut prior_generation = RevisionGate::default();
    assert!(matches!(
        prior_generation.apply(event(100, interactive_snapshot())),
        RevisionDecision::Apply(_)
    ));
    let replacement_baseline = event(1, CoreSnapshot::default());
    assert_eq!(
        prior_generation.apply(replacement_baseline.clone()),
        RevisionDecision::Ignore
    );

    assert_eq!(
        owner_stream_decision(OwnerStreamEvent::Changed),
        ConnectionGenerationDecision::Reconnect
    );
    let mut replacement_generation = RevisionGate::default();
    assert_eq!(
        replacement_generation.apply(replacement_baseline.clone()),
        RevisionDecision::Apply(replacement_baseline)
    );

    assert_eq!(
        owner_stream_decision(OwnerStreamEvent::Ended),
        ConnectionGenerationDecision::Reconnect
    );
}

#[test]
fn only_the_exact_unknown_method_name_selects_legacy_fallback() {
    assert_eq!(
        baseline_failure_for_error_name(Some("org.freedesktop.DBus.Error.UnknownMethod")),
        BaselineFailure::Legacy
    );
    assert_eq!(
        baseline_failure_for_error_name(Some("org.freedesktop.DBus.Error.UnknownInterface")),
        BaselineFailure::Reconnect
    );
    assert_eq!(
        baseline_failure_for_error_name(Some("UnknownMethod")),
        BaselineFailure::Reconnect
    );
    assert_eq!(
        baseline_failure_for_error_name(None),
        BaselineFailure::Reconnect
    );
}

#[test]
fn duplicate_reconciliation_does_not_publish_or_repaint() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let mut gate = RevisionGate::default();
    let mut intent = PassiveIntent::default();
    let baseline = event(7, interactive_snapshot());

    assert_eq!(
        apply_versioned(baseline.clone(), &mut gate, &sender, &mut intent, || {
            repaints.fetch_add(1, Ordering::SeqCst);
        },),
        VersionedHandling::Applied
    );
    assert!(receiver.take_latest().is_some());
    assert_eq!(
        apply_versioned(baseline, &mut gate, &sender, &mut intent, || {
            repaints.fetch_add(1, Ordering::SeqCst);
        },),
        VersionedHandling::Ignored
    );

    assert_eq!(receiver.take_latest(), None);
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn passive_intent_waits_for_a_versioned_passive_snapshot() {
    let mut intent = PassiveIntent::pending();

    intent.record_snapshot(&interactive_snapshot());
    assert!(intent.should_request());

    intent.record_snapshot(&CoreSnapshot {
        overlay_mode: OverlayMode::Passive,
        ..interactive_snapshot()
    });
    assert!(!intent.should_request());
}

#[test]
fn versioned_command_responses_do_not_publish_unversioned_snapshots() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let json = serde_json::to_string(&interactive_snapshot()).expect("snapshot serializes");

    assert_eq!(
        handle_command_response(CommandResponseMode::Versioned, &json, &sender, || {
            repaints.fetch_add(1, Ordering::SeqCst);
        }),
        None
    );
    assert_eq!(receiver.take_latest(), None);
    assert_eq!(repaints.load(Ordering::SeqCst), 0);

    assert_eq!(
        handle_command_response(CommandResponseMode::Legacy, &json, &sender, || {
            repaints.fetch_add(1, Ordering::SeqCst);
        }),
        Some(interactive_snapshot())
    );
    assert!(receiver.take_latest().is_some());
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn reconnect_backoff_is_exponential_bounded_and_resettable() {
    let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_millis(400));

    assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    assert_eq!(backoff.next_delay(), Duration::from_millis(200));
    assert_eq!(backoff.next_delay(), Duration::from_millis(400));
    assert_eq!(backoff.next_delay(), Duration::from_millis(400));

    backoff.reset();
    assert_eq!(backoff.next_delay(), Duration::from_millis(100));
}

#[test]
fn valid_json_is_sent_and_requests_a_repaint() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaint_counter = Arc::clone(&repaints);
    let json = serde_json::to_string(&interactive_snapshot()).expect("snapshot serializes");

    publish_json(&json, &sender, move || {
        repaint_counter.fetch_add(1, Ordering::SeqCst);
    });

    let update = receiver.take_latest().expect("snapshot is sent");
    assert_eq!(update.snapshot, interactive_snapshot());
    assert!(!update.passive_confirmed);
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn malformed_json_keeps_the_last_good_snapshot() {
    let (sender, receiver) = snapshot_channel();
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaint_counter = Arc::clone(&repaints);

    assert!(sender.publish(SnapshotUpdate::unconfirmed(interactive_snapshot())));
    assert!(receiver.take_latest().is_some());

    let decoded = publish_json("not json", &sender, move || {
        repaint_counter.fetch_add(1, Ordering::SeqCst);
    });

    assert!(decoded.is_none());
    // No new snapshot is published — last good interactive state remains.
    assert!(receiver.take_latest().is_none());
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn ui_drain_coalesces_snapshots_to_the_latest_value() {
    let (snapshot_sender, snapshot_receiver) = snapshot_channel();
    let (command_sender, _command_receiver) = command_channel();
    let client = SnapshotClient::from_channels(snapshot_receiver, command_sender);
    assert!(snapshot_sender.publish(SnapshotUpdate::unconfirmed(CoreSnapshot::default())));
    assert!(snapshot_sender.publish(SnapshotUpdate::unconfirmed(interactive_snapshot())));

    assert_eq!(
        client.take_latest(),
        Some(SnapshotUpdate::unconfirmed(interactive_snapshot()))
    );
    assert_eq!(client.take_latest(), None);
}

#[test]
fn source_coalescing_preserves_passive_confirmation_and_latest_value() {
    let (sender, receiver) = snapshot_channel();
    let passive = CoreSnapshot {
        overlay_mode: OverlayMode::Passive,
        ..interactive_snapshot()
    };
    assert!(sender.publish(SnapshotUpdate::confirmed(passive, true)));
    assert!(sender.publish(SnapshotUpdate::confirmed(interactive_snapshot(), false,)));

    let update = receiver.take_latest().expect("coalesced update is ready");
    assert_eq!(update.snapshot, interactive_snapshot());
    assert!(update.passive_confirmed);
    assert_eq!(receiver.take_latest(), None);
}

#[test]
fn passive_request_is_queued_without_dbus_work() {
    let (_snapshot_sender, snapshot_receiver) = snapshot_channel();
    let (command_sender, command_receiver) = command_channel();
    let client = SnapshotClient::from_channels(snapshot_receiver, command_sender);

    client.request_passive();

    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));
    assert!(intent.should_request());
}

#[test]
fn widget_actions_queue_only_coalesced_fixed_logical_commands() {
    let (_snapshot_sender, snapshot_receiver) = snapshot_channel();
    let (command_sender, command_receiver) = command_channel();
    let client = SnapshotClient::from_channels(snapshot_receiver, command_sender);

    client.reload_widget_settings();
    client.toggle_manual_stopwatch();
    client.reset_manual_stopwatch();

    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ReloadWidgetSettings)
    );
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ResetManualStopwatch)
    );
    assert_eq!(intent.next_widget_action(), None);
}

#[test]
fn renderer_command_ingress_has_one_fixed_wake_slot_and_coalesces_reloads() {
    assert_eq!(COMMAND_CHANNEL_CAPACITY, 1);
    let (command_sender, command_receiver) = command_channel();
    for _ in 0..1_000 {
        assert!(command_sender.send(ClientCommand::ReloadWidgetSettings));
    }

    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ReloadWidgetSettings)
    );
    assert_eq!(intent.next_widget_action(), None);
}

#[test]
fn reset_dominates_older_timer_work_and_later_toggles_reduce_by_parity() {
    let (command_sender, command_receiver) = command_channel();
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));

    for command in [
        ClientCommand::ResetManualStopwatch,
        ClientCommand::ToggleManualStopwatch,
        ClientCommand::ToggleManualStopwatch,
        ClientCommand::ToggleManualStopwatch,
    ] {
        assert!(command_sender.send(command));
    }

    assert!(intent.absorb_commands(&command_receiver));
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ResetManualStopwatch)
    );
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ToggleManualStopwatch)
    );
    assert_eq!(intent.next_widget_action(), None);
}

#[test]
fn consecutive_stopwatch_toggles_reduce_by_parity() {
    let (command_sender, command_receiver) = command_channel();
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));

    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));
    assert_eq!(intent.next_widget_action(), None);

    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    assert!(intent.absorb_commands(&command_receiver));
    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ToggleManualStopwatch)
    );
    assert_eq!(intent.next_widget_action(), None);
}

#[test]
fn dispatched_widget_action_is_not_retried_after_an_ambiguous_failure() {
    let (command_sender, command_receiver) = command_channel();
    assert!(command_sender.send(ClientCommand::ToggleManualStopwatch));
    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));

    assert_eq!(
        intent.next_widget_action(),
        Some(ClientCommand::ToggleManualStopwatch)
    );
    // Simulate a transport failure after dispatch: no response is recorded.
    assert_eq!(intent.next_widget_action(), None);
}

#[test]
fn saturated_renderer_commands_do_not_block_snapshot_delivery() {
    let (snapshot_sender, snapshot_receiver) = snapshot_channel();
    let (command_sender, _command_receiver) = command_channel();
    for _ in 0..1_000 {
        assert!(command_sender.send(ClientCommand::ReloadWidgetSettings));
    }

    assert!(snapshot_sender.publish(SnapshotUpdate::unconfirmed(CoreSnapshot::default())));
    assert!(snapshot_sender.publish(SnapshotUpdate::unconfirmed(interactive_snapshot())));
    assert_eq!(
        snapshot_receiver.take_latest(),
        Some(SnapshotUpdate::unconfirmed(interactive_snapshot()))
    );
}

#[test]
fn failed_dbus_request_does_not_consume_the_passive_intent() {
    let (command_sender, command_receiver) = command_channel();
    assert!(command_sender.send(ClientCommand::SetPassive));
    let mut intent = PassiveIntent::default();
    assert!(intent.absorb_commands(&command_receiver));
    assert!(intent.should_request());

    intent.record_response(Err(()));

    assert!(intent.should_request());
}

#[test]
fn only_a_confirmed_passive_response_clears_the_worker_intent() {
    let mut intent = PassiveIntent::pending();

    let interactive = interactive_snapshot();
    intent.record_response(Ok(&interactive));
    assert!(intent.should_request());

    let passive = CoreSnapshot {
        overlay_mode: OverlayMode::Passive,
        ..interactive
    };
    intent.record_response(Ok(&passive));
    assert!(!intent.should_request());
}

#[test]
fn dropping_the_client_signals_shutdown_and_closes_commands() {
    let (_snapshot_sender, snapshot_receiver) = snapshot_channel();
    let (command_sender, command_receiver) = command_channel();
    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
    let client = SnapshotClient::from_channels_with_shutdown(
        snapshot_receiver,
        command_sender,
        shutdown_sender,
    );

    drop(client);

    assert!(*shutdown_receiver.borrow());
    assert!(!PassiveIntent::default().absorb_commands(&command_receiver));
}
