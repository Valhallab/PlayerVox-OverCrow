use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use overcrow_logging::{Component, LoggerRuntime};

use super::{
    MarketClient, MarketCommand, MarketSnapshot,
    client::{MarketBackend, normalize_command, publish_if_current, retry_policy},
    model::{
        MarketItemDetail, MarketItemSummary, MarketOrder, TradeSide, TraderPresence,
        format_trade_line, format_whisper_line,
    },
};
use crate::warframe::model::STRING_MAX_CHARS;
use crate::{runtime::latest_channel, warframe::http::HttpError};

fn detail(slug: &str, platinum: u32) -> MarketItemDetail {
    MarketItemDetail {
        name: "Test Item".to_owned(),
        slug: slug.to_owned(),
        lowest_sell: Some(platinum),
        highest_buy: None,
        order_count: 1,
        top_sells: vec![MarketOrder {
            side: TradeSide::Sell,
            platinum,
            trader: "TestTrader".to_owned(),
            presence: TraderPresence::Online,
        }],
        top_buys: Vec::new(),
    }
}

#[test]
fn unchanged_market_frame_does_not_publish_or_replace_the_snapshot() {
    let (publisher, receiver) = latest_channel(super::MarketSnapshot::default());
    let enabled = AtomicBool::new(true);
    let generation = AtomicU64::new(0);
    let shutdown = AtomicBool::new(false);
    let repaints = AtomicUsize::new(0);
    let snapshot = super::MarketSnapshot {
        status: Some("loading".to_owned()),
        ..super::MarketSnapshot::default()
    };

    assert!(publish_if_current(
        &publisher,
        &enabled,
        &generation,
        &shutdown,
        0,
        &snapshot,
        &|| {
            repaints.fetch_add(1, Ordering::SeqCst);
        },
    ));
    let first = receiver.take_latest().expect("changed frame is ready");

    assert!(!publish_if_current(
        &publisher,
        &enabled,
        &generation,
        &shutdown,
        0,
        &snapshot,
        &|| {
            repaints.fetch_add(1, Ordering::SeqCst);
        },
    ));
    assert!(receiver.take_latest().is_none());
    assert!(Arc::ptr_eq(&first.value, &publisher.current().value));
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

fn wait_for_snapshot(
    client: &MarketClient,
    mut condition: impl FnMut(&MarketSnapshot) -> bool,
) -> Arc<MarketSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(snapshot) = client.take_latest()
            && condition(&snapshot.value)
        {
            return snapshot.value;
        }
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn warframe_diagnostic_market_omits_private_failure_and_reports_recovery() {
    let temp = tempfile::tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let client = MarketClient::with_backend_and_logger(
        ScriptedBackend {
            calls: Arc::new(AtomicUsize::new(0)),
            orders: VecDeque::from([
                Err(HttpError::Transport("private market detail".to_owned())),
                Ok(detail("test_item", 10)),
            ]),
        },
        log_runtime.logger(),
    );
    client.set_enabled(true);
    client.send(MarketCommand::Select("test_item".to_owned()));
    wait_for_snapshot(&client, |snapshot| snapshot.error.is_some());
    client.send(MarketCommand::Select("test_item".to_owned()));
    wait_for_snapshot(&client, |snapshot| snapshot.selected.is_some());
    drop(client);
    drop(log_runtime);

    let contents =
        std::fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
    assert_eq!(contents.matches("widget_provider_failed").count(), 1);
    assert!(contents.contains(
        "widget_provider_failed widget=warframe_market provider=warframe_market category=transport"
    ));
    assert!(
        contents
            .contains("widget_provider_recovered widget=warframe_market provider=warframe_market")
    );
    assert!(!contents.contains("private market detail"));
}

struct ScriptedBackend {
    calls: Arc<AtomicUsize>,
    orders: VecDeque<Result<MarketItemDetail, HttpError>>,
}

impl MarketBackend for ScriptedBackend {
    fn search(&mut self, _query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    fn orders(&mut self, _slug: &str) -> Result<MarketItemDetail, HttpError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.orders.pop_front().expect("scripted order response")
    }
}

struct BlockOnSecondOrderBackend {
    calls: Arc<AtomicUsize>,
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl MarketBackend for BlockOnSecondOrderBackend {
    fn search(&mut self, _query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        Ok(Vec::new())
    }

    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 2 {
            self.started.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(detail(slug, call as u32))
    }
}

struct BlockingBackend {
    calls: Arc<AtomicUsize>,
    started: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
    finished: Option<mpsc::Sender<()>>,
}

struct BlockFirstRecordingBackend {
    calls: usize,
    first_started: mpsc::Sender<()>,
    release_first: mpsc::Receiver<()>,
    slugs: mpsc::Sender<String>,
}

impl MarketBackend for BlockFirstRecordingBackend {
    fn search(&mut self, _query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        Ok(Vec::new())
    }

    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError> {
        self.calls += 1;
        self.slugs.send(slug.to_owned()).unwrap();
        if self.calls == 1 {
            self.first_started.send(()).unwrap();
            self.release_first.recv().unwrap();
        }
        Ok(detail(slug, self.calls as u32))
    }
}

impl MarketBackend for BlockingBackend {
    fn search(&mut self, _query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        Ok(Vec::new())
    }

    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(started) = self.started.take() {
            started.send(()).unwrap();
        }
        self.release.recv().unwrap();
        if let Some(finished) = self.finished.take() {
            finished.send(()).unwrap();
        }
        Ok(detail(slug, 42))
    }
}

#[test]
fn trade_lines_are_public_chat_templates() {
    assert_eq!(
        format_trade_line(TradeSide::Sell, "Valkyr Prime Set", 90),
        "WTS Valkyr Prime Set 90p"
    );
    assert_eq!(
        format_trade_line(TradeSide::Buy, "Forma", 12),
        "WTB Forma 12p"
    );
}

#[test]
fn whisper_lines_target_a_named_trader() {
    // Contact a seller because you want to buy.
    assert_eq!(
        format_whisper_line(TradeSide::Buy, "SellerOne", "Valkyr Prime Set", 75),
        "/w SellerOne Hi, WTB Valkyr Prime Set for 75p"
    );
    // Contact a buyer because you want to sell.
    assert_eq!(
        format_whisper_line(TradeSide::Sell, "BuyerTwo", "Forma", 12),
        "/w BuyerTwo Hi, WTS Forma for 12p"
    );
    // Injection hardening: no multi-line or slash injection via trader name.
    let line = format_whisper_line(TradeSide::Buy, "Bad\n/w Eve", "Forma", 1);
    assert!(!line.contains('\n'));
    assert_eq!(line.matches("/w").count(), 1);
}

#[test]
fn repeated_refresh_requests_coalesce_while_one_is_in_flight() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let client = MarketClient::with_backend(BlockOnSecondOrderBackend {
        calls: Arc::clone(&calls),
        started: started_tx,
        release: release_rx,
    });
    client.set_enabled(true);
    client.send(MarketCommand::Select("test_item".to_owned()));
    wait_for_snapshot(&client, |snapshot| snapshot.selected.is_some());

    for _ in 0..32 {
        client.send(MarketCommand::RefreshSelected);
    }
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    release_tx.send(()).unwrap();
    wait_for_snapshot(&client, |snapshot| {
        snapshot.selected.as_ref().unwrap().lowest_sell == Some(2)
    });
}

#[test]
fn disabled_market_client_performs_no_backend_work() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = MarketClient::with_backend(ScriptedBackend {
        calls: Arc::clone(&calls),
        orders: VecDeque::new(),
    });

    client.send(MarketCommand::Search("test".to_owned()));
    client.send(MarketCommand::Select("test_item".to_owned()));
    client.send(MarketCommand::RefreshSelected);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn search_commands_are_trimmed_and_unicode_bounded_before_enqueue() {
    assert_eq!(
        normalize_command(MarketCommand::Search("  duviri arcane  ".to_owned())),
        Some(MarketCommand::Search("duviri arcane".to_owned()))
    );

    let normalized = normalize_command(MarketCommand::Search(format!(
        "  {}  ",
        "é".repeat(STRING_MAX_CHARS + 10)
    )));
    let Some(MarketCommand::Search(query)) = normalized else {
        panic!("search command should be retained");
    };
    assert_eq!(query.chars().count(), STRING_MAX_CHARS);
    assert!(query.ends_with('…'));
    assert!(
        query
            .chars()
            .take(STRING_MAX_CHARS - 1)
            .all(|character| character == 'é')
    );
}

#[test]
fn select_commands_are_validated_before_enqueue() {
    assert_eq!(
        normalize_command(MarketCommand::Select("valid_slug-2".to_owned())),
        Some(MarketCommand::Select("valid_slug-2".to_owned()))
    );
    assert_eq!(
        normalize_command(MarketCommand::Select("Bad/../slug".to_owned())),
        None
    );
    assert_eq!(
        normalize_command(MarketCommand::Select("a".repeat(STRING_MAX_CHARS + 1))),
        None
    );
}

#[test]
fn rejected_select_payloads_do_not_consume_bounded_queue_slots() {
    let (first_started_tx, first_started_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let (slugs_tx, slugs_rx) = mpsc::channel();
    let client = MarketClient::with_backend(BlockFirstRecordingBackend {
        calls: 0,
        first_started: first_started_tx,
        release_first: release_first_rx,
        slugs: slugs_tx,
    });
    client.set_enabled(true);
    client.send(MarketCommand::Select("first_item".to_owned()));
    first_started_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        slugs_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "first_item"
    );

    for index in 0..8 {
        let rejected = if index % 2 == 0 {
            "Bad/../slug".to_owned()
        } else {
            "a".repeat(STRING_MAX_CHARS + 1)
        };
        client.send(MarketCommand::Select(rejected));
    }
    client.send(MarketCommand::Select("second_item".to_owned()));
    release_first_tx.send(()).unwrap();

    assert_eq!(
        slugs_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "second_item"
    );
}

#[test]
fn disabling_during_a_request_suppresses_its_result() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let client = MarketClient::with_backend(BlockingBackend {
        calls,
        started: Some(started_tx),
        release: release_rx,
        finished: Some(finished_tx),
    });
    client.set_enabled(true);
    client.send(MarketCommand::Select("test_item".to_owned()));
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    client.set_enabled(false);
    release_tx.send(()).unwrap();
    finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let snapshot = client
        .take_latest()
        .expect("disabling publishes the cleared status")
        .value;

    assert!(snapshot.selected.is_none());
    assert!(snapshot.status.is_none());
}

#[test]
fn rate_limit_keeps_last_good_orders_and_sets_capped_deadline() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = MarketClient::with_backend_and_clock(
        ScriptedBackend {
            calls,
            orders: VecDeque::from([
                Ok(detail("test_item", 10)),
                Err(HttpError::Status {
                    code: 429,
                    retry_after: Some(Duration::from_secs(999)),
                }),
            ]),
        },
        || 1_000,
    );
    client.set_enabled(true);
    client.send(MarketCommand::Select("test_item".to_owned()));
    let last_good = wait_for_snapshot(&client, |snapshot| snapshot.selected.is_some());

    client.send(MarketCommand::RefreshSelected);
    let failed = wait_for_snapshot(&client, |snapshot| snapshot.error.is_some());

    assert_eq!(failed.selected, last_good.selected);
    assert_eq!(
        failed.selected_fetched_at_secs,
        last_good.selected_fetched_at_secs
    );
    assert_eq!(failed.next_refresh_at_secs, 1_300);
}

#[test]
fn ordinary_retry_policy_doubles_to_the_fixed_maximum() {
    let error = HttpError::Transport("offline".to_owned());
    let mut backoff = Duration::from_secs(5);

    for expected_delay in [5, 10, 20, 40, 80, 120, 120] {
        let policy = retry_policy(&error, backoff);
        assert_eq!(policy.delay, Duration::from_secs(expected_delay));
        backoff = policy.next_backoff;
    }
}

#[test]
fn drop_does_not_drain_queued_network_commands() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let client = MarketClient::with_backend(BlockingBackend {
        calls: Arc::clone(&calls),
        started: Some(started_tx),
        release: release_rx,
        finished: None,
    });
    client.set_enabled(true);
    client.send(MarketCommand::Select("test_item".to_owned()));
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    for _ in 0..8 {
        client.send(MarketCommand::Select("test_item".to_owned()));
    }

    let (dropped_tx, dropped_rx) = mpsc::channel();
    thread::spawn(move || {
        drop(client);
        dropped_tx.send(()).unwrap();
    });
    assert!(dropped_rx.recv_timeout(Duration::from_millis(50)).is_err());
    release_tx.send(()).unwrap();

    dropped_rx.recv_timeout(Duration::from_millis(250)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
