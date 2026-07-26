use std::{
    fmt::Write as _,
    io::Read,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ControlLogSnapshot, ControlSnapshot};

pub const MAX_SUPPORT_DESCRIPTION_BYTES: usize = 2_000;
pub const SUPPORT_REPORT_SCHEMA_VERSION: u32 = 1;
pub const SUPPORT_REPORT_ENDPOINT: &str = "https://api.playervox.com/api/v1/overcrow/reports";
const MAX_SUPPORT_REPORT_BYTES: usize = 320 * 1024;
const MAX_SUPPORT_REQUEST_BYTES: usize = 384 * 1024;
const MAX_SUPPORT_RESPONSE_BYTES: u64 = 8 * 1024;
const SUPPORT_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const SUPPORT_USER_AGENT: &str = concat!(
    "PlayerVox-OverCrow/",
    env!("CARGO_PKG_VERSION"),
    " (support report)"
);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSupportReport {
    pub schema_version: u32,
    pub report_id: String,
    pub created_at: String,
    pub content: String,
    pub logs_included: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSupportReceipt {
    pub reference: String,
    pub received_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportReportError {
    DescriptionRequired,
    DescriptionInvalid,
    DescriptionTooLong,
    InvalidSnapshot,
    InvalidReport,
    RequestTooLarge,
    SubmissionBusy,
    ReportUnavailable,
    Timeout,
    NetworkUnavailable,
    Rejected,
    Conflict,
    RateLimited,
    InvalidResponse,
}

impl SupportReportError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::DescriptionRequired => "support_description_required",
            Self::DescriptionInvalid => "support_description_invalid",
            Self::DescriptionTooLong => "support_description_too_long",
            Self::InvalidSnapshot => "support_snapshot_invalid",
            Self::InvalidReport => "support_report_invalid",
            Self::RequestTooLarge => "support_request_too_large",
            Self::SubmissionBusy => "support_submission_busy",
            Self::ReportUnavailable => "support_report_unavailable",
            Self::Timeout => "support_submission_timeout",
            Self::NetworkUnavailable => "support_network_unavailable",
            Self::Rejected => "support_submission_rejected",
            Self::Conflict => "support_report_conflict",
            Self::RateLimited => "support_rate_limited",
            Self::InvalidResponse => "support_response_invalid",
        }
    }
}

pub fn prepare_support_report(
    snapshot: &ControlSnapshot,
    logs: Option<&ControlLogSnapshot>,
    description: &str,
) -> Result<ControlSupportReport, SupportReportError> {
    let report_id = format!("oc-{}", Uuid::new_v4());
    prepare_support_report_at(snapshot, logs, description, SystemTime::now(), &report_id)
}

fn prepare_support_report_at(
    snapshot: &ControlSnapshot,
    logs: Option<&ControlLogSnapshot>,
    description: &str,
    created_at: SystemTime,
    report_id: &str,
) -> Result<ControlSupportReport, SupportReportError> {
    let description = validated_description(description)?;
    if !snapshot.has_valid_wire_bounds()
        || logs.is_some_and(|snapshot| !snapshot.has_valid_wire_bounds())
    {
        return Err(SupportReportError::InvalidSnapshot);
    }
    validate_report_id(report_id)?;

    let created_at = timestamp_at(created_at);
    let content = render_report(snapshot, logs, &description, report_id, &created_at)?;
    Ok(ControlSupportReport {
        schema_version: SUPPORT_REPORT_SCHEMA_VERSION,
        report_id: report_id.to_owned(),
        created_at,
        content,
        logs_included: logs.is_some(),
    })
}

pub struct SupportReportClient {
    endpoint: String,
    agent: ureq::Agent,
}

impl Default for SupportReportClient {
    fn default() -> Self {
        Self::new(SUPPORT_REPORT_ENDPOINT, SUPPORT_REQUEST_TIMEOUT, true)
    }
}

impl SupportReportClient {
    fn new(endpoint: &str, timeout: Duration, https_only: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(https_only)
            .proxy(None)
            .build()
            .into();
        Self {
            endpoint: endpoint.to_owned(),
            agent,
        }
    }

    pub fn submit(
        &self,
        report: &ControlSupportReport,
    ) -> Result<ControlSupportReceipt, SupportReportError> {
        validate_report(report)?;
        let payload = SupportReportPayload {
            report_id: &report.report_id,
            schema_version: report.schema_version,
            app_version: env!("CARGO_PKG_VERSION"),
            created_at: &report.created_at,
            logs_included: report.logs_included,
            report_body: &report.content,
        };
        let body = serde_json::to_vec(&payload).map_err(|_| SupportReportError::InvalidReport)?;
        if body.len() > MAX_SUPPORT_REQUEST_BYTES {
            return Err(SupportReportError::RequestTooLarge);
        }

        let mut response = self
            .agent
            .post(&self.endpoint)
            .header("User-Agent", SUPPORT_USER_AGENT)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(body.as_slice())
            .map_err(|error| match error {
                ureq::Error::Timeout(_) => SupportReportError::Timeout,
                _ => SupportReportError::NetworkUnavailable,
            })?;

        match response.status().as_u16() {
            200 | 201 => {}
            409 => return Err(SupportReportError::Conflict),
            429 => return Err(SupportReportError::RateLimited),
            _ => return Err(SupportReportError::Rejected),
        }

        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_SUPPORT_RESPONSE_BYTES.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    SupportReportError::Timeout
                } else {
                    SupportReportError::InvalidResponse
                }
            })?;
        if body.len() as u64 > MAX_SUPPORT_RESPONSE_BYTES {
            return Err(SupportReportError::InvalidResponse);
        }

        let response: SupportReportResponse =
            serde_json::from_slice(&body).map_err(|_| SupportReportError::InvalidResponse)?;
        validate_receipt(&response)?;
        Ok(ControlSupportReceipt {
            reference: response.reference,
            received_at: response.received_at,
        })
    }

    #[cfg(test)]
    fn for_tests(endpoint: &str, timeout: Duration) -> Self {
        Self::new(endpoint, timeout, false)
    }
}

#[derive(Serialize)]
struct SupportReportPayload<'a> {
    report_id: &'a str,
    schema_version: u32,
    app_version: &'static str,
    created_at: &'a str,
    logs_included: bool,
    report_body: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportReportResponse {
    reference: String,
    received_at: String,
}

fn validate_report(report: &ControlSupportReport) -> Result<(), SupportReportError> {
    if report.schema_version != SUPPORT_REPORT_SCHEMA_VERSION
        || report.content.is_empty()
        || report.content.len() > MAX_SUPPORT_REPORT_BYTES
        || DateTime::parse_from_rfc3339(&report.created_at).is_err()
    {
        return Err(SupportReportError::InvalidReport);
    }
    validate_report_id(&report.report_id)
}

fn validate_report_id(report_id: &str) -> Result<(), SupportReportError> {
    let uuid = report_id
        .strip_prefix("oc-")
        .and_then(|value| Uuid::parse_str(value).ok());
    if uuid.is_none() || report_id.len() > 80 {
        return Err(SupportReportError::InvalidReport);
    }
    Ok(())
}

fn validate_receipt(response: &SupportReportResponse) -> Result<(), SupportReportError> {
    if Uuid::parse_str(&response.reference).is_err()
        || DateTime::parse_from_rfc3339(&response.received_at).is_err()
    {
        return Err(SupportReportError::InvalidResponse);
    }
    Ok(())
}

fn validated_description(description: &str) -> Result<String, SupportReportError> {
    if description.len() > MAX_SUPPORT_DESCRIPTION_BYTES {
        return Err(SupportReportError::DescriptionTooLong);
    }
    if description
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(SupportReportError::DescriptionInvalid);
    }
    let normalized = description.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return Err(SupportReportError::DescriptionRequired);
    }
    Ok(normalized.to_owned())
}

fn render_report(
    snapshot: &ControlSnapshot,
    logs: Option<&ControlLogSnapshot>,
    description: &str,
    report_id: &str,
    created_at: &str,
) -> Result<String, SupportReportError> {
    let mut output = String::with_capacity(8 * 1024);
    output.push_str("# PlayerVox OverCrow support report\n\n");
    output.push_str(
        "Review this report before sending it. Nothing is sent until you explicitly confirm.\n\n",
    );
    output.push_str("## Report\n\n");
    let _ = writeln!(output, "- Report ID: {report_id}");
    let _ = writeln!(output, "- Created at: {created_at}");
    let _ = writeln!(output, "- OverCrow: {}", env!("CARGO_PKG_VERSION"));

    output.push_str("\n## What happened\n\n");
    for line in description.lines() {
        output.push_str("> ");
        output.push_str(line);
        output.push('\n');
    }

    output.push_str("\n## Environment\n\n");
    let _ = writeln!(
        output,
        "- Operating system: {}",
        one_line(&snapshot.compatibility.operating_system)
    );
    let _ = writeln!(
        output,
        "- Session: {}",
        serialized_code(&snapshot.compatibility.session)
    );
    let _ = writeln!(
        output,
        "- Desktop: {}",
        serialized_code(&snapshot.compatibility.desktop)
    );
    let _ = writeln!(
        output,
        "- Compatibility: {}",
        serialized_code(&snapshot.compatibility.status)
    );
    let _ = writeln!(
        output,
        "- Compatibility reason: {}",
        serialized_code(&snapshot.compatibility.reason)
    );
    let _ = writeln!(
        output,
        "- Activation allowed: {}",
        snapshot.compatibility.activation_allowed
    );
    let _ = writeln!(
        output,
        "- Lifecycle: {}",
        serialized_code(&snapshot.lifecycle)
    );
    let _ = writeln!(
        output,
        "- Runtime enabled: {}",
        snapshot.master_switch_checked
    );

    let mut selected_app_ids = snapshot
        .games
        .iter()
        .filter(|game| game.selected)
        .map(|game| game.app_id)
        .collect::<Vec<_>>();
    selected_app_ids.sort_unstable();
    selected_app_ids.dedup();
    let selected_app_ids = if selected_app_ids.is_empty() {
        "none".to_owned()
    } else {
        selected_app_ids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(output, "- Selected Steam App IDs: {selected_app_ids}");
    let _ = writeln!(
        output,
        "- Selected native games: {}",
        snapshot.manual_games.len()
    );

    output.push_str("\n## Diagnostics\n\n");
    if snapshot.diagnostics.is_empty() {
        output.push_str("- No diagnostics were available.\n");
    } else {
        for diagnostic in &snapshot.diagnostics {
            let _ = writeln!(
                output,
                "- {}: {} (detail omitted)",
                one_line(&diagnostic.label),
                serialized_code(&diagnostic.level)
            );
        }
    }

    output.push_str("\n## Sanitized logs\n\n");
    match logs {
        Some(logs) if logs.lines.is_empty() => {
            output.push_str("No OverCrow log lines were available.\n");
        }
        Some(logs) => {
            if logs.truncated {
                output.push_str("The log reader retained only the newest bounded lines.\n\n");
            }
            for line in &logs.lines {
                output.push_str("    ");
                output.push_str(line);
                output.push('\n');
            }
        }
        None => output.push_str("Logs were not included by the user.\n"),
    }
    if output.len() > MAX_SUPPORT_REPORT_BYTES {
        return Err(SupportReportError::InvalidSnapshot);
    }
    Ok(output)
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn serialized_code(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(code)) => code,
        _ => "unknown".to_owned(),
    }
}

fn timestamp_at(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::{Duration, UNIX_EPOCH},
    };
    use uuid::Uuid;

    use crate::{
        CompatibilityReason, CompatibilityStatus, ControlCompatibility, ControlDiagnostic,
        ControlGame, ControlLifecycle, ControlLogSnapshot, ControlManualGame,
        ControlOperationState, ControlSnapshot, DesktopEnvironment, DiagnosticLevelCode,
        DisplaySession,
    };

    use super::{
        MAX_SUPPORT_DESCRIPTION_BYTES, SUPPORT_REPORT_ENDPOINT, SupportReportClient,
        SupportReportError, prepare_support_report, prepare_support_report_at,
    };

    fn snapshot() -> ControlSnapshot {
        ControlSnapshot {
            schema_version: 1,
            compatibility: ControlCompatibility {
                operating_system: "Arch Linux".to_owned(),
                session: DisplaySession::Wayland,
                desktop: DesktopEnvironment::Hyprland,
                status: CompatibilityStatus::Supported,
                reason: CompatibilityReason::HyprlandWayland,
                activation_allowed: true,
            },
            lifecycle: ControlLifecycle::Enabled,
            master_switch_enabled: true,
            master_switch_checked: true,
            selection_editing_enabled: true,
            shortcut: "SUPER+ALT+O".to_owned(),
            operations: ControlOperationState::default(),
            games: vec![
                ControlGame {
                    app_id: 4242,
                    name: "Private game title".to_owned(),
                    selected: true,
                },
                ControlGame {
                    app_id: 9999,
                    name: "Other private title".to_owned(),
                    selected: false,
                },
            ],
            manual_games: vec![ControlManualGame {
                id: "local.0123456789abcdef0123456789abcdef".to_owned(),
                name: "Private native game".to_owned(),
                executable: "/home/player/private/game".to_owned(),
            }],
            notices: Vec::new(),
            diagnostics: vec![ControlDiagnostic {
                label: "Settings path".to_owned(),
                detail: "Using /home/player/.config/overcrow/settings.json.".to_owned(),
                level: DiagnosticLevelCode::Warning,
            }],
        }
    }

    fn logs() -> ControlLogSnapshot {
        ControlLogSnapshot {
            schema_version: 1,
            lines: vec![
                "2026-07-24T10:00:00.000Z INFO core process_started".to_owned(),
                "2026-07-24T10:00:01.000Z WARN overlay frame_late count=1".to_owned(),
            ],
            truncated: false,
        }
    }

    fn report() -> super::ControlSupportReport {
        prepare_support_report_at(
            &snapshot(),
            Some(&logs()),
            "The overlay froze.",
            UNIX_EPOCH,
            "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d",
        )
        .expect("support report")
    }

    #[test]
    fn report_contains_only_bounded_support_fields_and_sanitized_logs() {
        let report = report();

        assert_eq!(report.schema_version, 1);
        assert_eq!(report.created_at, "1970-01-01T00:00:00.000Z");
        assert_eq!(report.report_id, "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d");
        assert!(report.content.contains("> The overlay froze."));
        assert!(report.content.contains("- OverCrow: 0.1.0-pre-alpha.3"));
        assert!(report.content.contains("- Operating system: Arch Linux"));
        assert!(report.content.contains("- Selected Steam App IDs: 4242"));
        assert!(report.content.contains("- Selected native games: 1"));
        assert!(
            report
                .content
                .contains("- Settings path: warning (detail omitted)")
        );
        assert!(report.content.contains("WARN overlay frame_late count=1"));
        for private in [
            "Private game title",
            "Other private title",
            "Private native game",
            "local.0123456789abcdef0123456789abcdef",
            "/home/player",
        ] {
            assert!(!report.content.contains(private), "leaked {private}");
        }
    }

    #[test]
    fn description_validation_is_exact_and_utf8_bounded() {
        for invalid in ["", " \n\t ", "contains\u{0}control"] {
            assert!(
                prepare_support_report_at(
                    &snapshot(),
                    None,
                    invalid,
                    UNIX_EPOCH,
                    "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d",
                )
                .is_err()
            );
        }
        let exact = "é".repeat(MAX_SUPPORT_DESCRIPTION_BYTES / 2);
        assert!(
            prepare_support_report_at(
                &snapshot(),
                None,
                &exact,
                UNIX_EPOCH,
                "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d",
            )
            .is_ok()
        );
        let oversized = format!("{exact}a");
        assert_eq!(
            prepare_support_report_at(
                &snapshot(),
                None,
                &oversized,
                UNIX_EPOCH,
                "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d",
            )
            .expect_err("oversized description"),
            SupportReportError::DescriptionTooLong
        );
    }

    #[test]
    fn omitting_logs_is_explicit() {
        let report = prepare_support_report_at(
            &snapshot(),
            None,
            "No logs please.",
            UNIX_EPOCH,
            "oc-7f69c535-6d20-47fb-bc98-df4db4f7071d",
        )
        .expect("support report");

        assert!(!report.logs_included);
        assert!(
            report
                .content
                .contains("Logs were not included by the user.")
        );
        assert!(!report.content.contains("saved"));
    }

    #[test]
    fn generated_report_ids_are_unique_api_compatible_uuids() {
        let first =
            prepare_support_report(&snapshot(), None, "First issue.").expect("first report");
        let second =
            prepare_support_report(&snapshot(), None, "Second issue.").expect("second report");

        assert_ne!(first.report_id, second.report_id);
        for report_id in [first.report_id, second.report_id] {
            let uuid = report_id.strip_prefix("oc-").expect("report prefix");
            assert_eq!(
                Uuid::parse_str(uuid)
                    .expect("report UUID")
                    .get_version_num(),
                4
            );
        }
        assert_eq!(
            SUPPORT_REPORT_ENDPOINT,
            "https://api.playervox.com/api/v1/overcrow/reports"
        );
    }

    #[test]
    fn successful_submission_has_the_exact_api_shape() {
        let (endpoint, request_rx) = spawn_server(
            201,
            r#"{"reference":"bfaf03ce-5471-4739-a145-1ca24f215f1b","received_at":"2026-07-25T10:00:00Z"}"#,
        );
        let client = SupportReportClient::for_tests(&endpoint, Duration::from_secs(1));

        let receipt = client.submit(&report()).expect("submitted report");
        let request = request_rx.recv().expect("request");
        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let payload: serde_json::Value = serde_json::from_str(body).expect("JSON payload");

        assert_eq!(receipt.reference, "bfaf03ce-5471-4739-a145-1ca24f215f1b");
        assert!(request.starts_with("POST /reports HTTP/1.1\r\n"));
        let lowercase_request = request.to_ascii_lowercase();
        assert!(lowercase_request.contains("\r\ncontent-type: application/json\r\n"));
        assert!(!lowercase_request.contains("\r\nauthorization:"));
        assert!(!lowercase_request.contains("\r\ncookie:"));
        assert_eq!(
            payload.as_object().expect("object").keys().count(),
            6,
            "the API payload must not grow implicitly"
        );
        assert_eq!(payload["report_id"], report().report_id);
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["app_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(payload["report_body"], report().content);
    }

    #[test]
    fn redirects_conflicts_rate_limits_and_bad_responses_fail_closed() {
        for (status, body, expected) in [
            (302, "", SupportReportError::Rejected),
            (409, "{}", SupportReportError::Conflict),
            (429, "{}", SupportReportError::RateLimited),
            (201, "{}", SupportReportError::InvalidResponse),
            (
                201,
                r#"{"reference":"not-a-uuid","received_at":"2026-07-25T10:00:00Z"}"#,
                SupportReportError::InvalidResponse,
            ),
        ] {
            let (endpoint, _request_rx) = spawn_server(status, body);
            let client = SupportReportClient::for_tests(&endpoint, Duration::from_secs(1));
            assert_eq!(client.submit(&report()), Err(expected));
        }
    }

    #[test]
    fn oversized_response_is_rejected_before_full_allocation() {
        let oversized = "x".repeat(9 * 1024);
        let (endpoint, _request_rx) = spawn_server(201, &oversized);
        let client = SupportReportClient::for_tests(&endpoint, Duration::from_secs(1));

        assert_eq!(
            client.submit(&report()),
            Err(SupportReportError::InvalidResponse)
        );
    }

    #[test]
    fn stalled_server_is_bounded_by_the_request_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let endpoint = format!("http://{}/reports", listener.local_addr().expect("address"));
        thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("request");
            thread::sleep(Duration::from_millis(200));
        });
        let client = SupportReportClient::for_tests(&endpoint, Duration::from_millis(30));

        assert_eq!(client.submit(&report()), Err(SupportReportError::Timeout));
    }

    fn spawn_server(status: u16, response_body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let endpoint = format!("http://{}/reports", listener.local_addr().expect("address"));
        let response_body = response_body.to_owned();
        let (request_tx, request_rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("read timeout");
            let request = read_request(&mut stream);
            request_tx.send(request).expect("request receiver");
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("response");
        });
        (endpoint, request_rx)
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = stream.read(&mut buffer).expect("request bytes");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let content_length = String::from_utf8_lossy(&request[..header_end])
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8(request).expect("UTF-8 request")
    }
}
