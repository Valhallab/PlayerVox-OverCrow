//! Bounded HTTPS client for Warframe public APIs.
//!
//! Security properties:
//! - HTTPS only, fixed host allowlist (no open redirects off-host)
//! - Response body size cap before full allocation
//! - Global request timeout
//! - Identifiable User-Agent (no cookies, no auth)

use std::{fmt, io::Read, time::Duration};

use url::{Position, Url};

use crate::runtime::widget_diagnostics::FailureCategory;

pub const USER_AGENT: &str = concat!(
    "OverCrow/",
    env!("CARGO_PKG_VERSION"),
    " (Warframe widgets; safe public data only)"
);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_ERROR_MAX_CHARS: usize = 180;
const RETRY_AFTER_MAX_SECS: u64 = 300;
pub const WORLDSTATE_HOST: &str = "api.warframe.com";
pub const MARKET_HOST: &str = "api.warframe.market";
pub const WORLDSTATE_MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const MARKET_MAX_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpError {
    InvalidUrl,
    Timeout,
    Transport(String),
    Status {
        code: u16,
        retry_after: Option<Duration>,
    },
    BodyTooLarge {
        maximum: u64,
    },
    Read(String),
    Parse(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => formatter.write_str("invalid or disallowed URL"),
            Self::Timeout => formatter.write_str("HTTP request timed out"),
            Self::Transport(message) => write!(formatter, "HTTP transport failed: {message}"),
            Self::Status { code, .. } => write!(formatter, "HTTP status {code}"),
            Self::BodyTooLarge { maximum } => {
                write!(formatter, "response too large (maximum {maximum} bytes)")
            }
            Self::Read(message) => write!(formatter, "response read failed: {message}"),
            Self::Parse(message) => write!(formatter, "response parse failed: {message}"),
        }
    }
}

impl std::error::Error for HttpError {}

pub fn validate_https_url(url: &str, allowed_host: &str) -> Result<Url, HttpError> {
    let parsed = Url::parse(url).map_err(|_| HttpError::InvalidUrl)?;
    let host = parsed.host_str().ok_or(HttpError::InvalidUrl)?;
    let has_userinfo = !parsed[Position::BeforeUsername..Position::BeforeHost].is_empty()
        || raw_authority(url).is_some_and(|authority| authority.contains('@'));
    if parsed.scheme() != "https"
        || has_userinfo
        || !host.eq_ignore_ascii_case(allowed_host)
        || parsed.port_or_known_default() != Some(443)
    {
        return Err(HttpError::InvalidUrl);
    }
    Ok(parsed)
}

fn raw_authority(url: &str) -> Option<&str> {
    let scheme_end = url.find(':')?;
    let after_scheme = url.get(scheme_end + 1..)?.strip_prefix("//")?;
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    Some(&after_scheme[..authority_end])
}

/// GET `url` only if its host is exactly `allowed_host`, with a size cap.
pub fn https_get_allowlisted(
    url: &str,
    allowed_host: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, HttpError> {
    validate_https_url(url, allowed_host)?;
    get_with_agent(&http_agent(), url, max_bytes)
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_with_agent(agent: &ureq::Agent, url: &str, max_bytes: u64) -> Result<Vec<u8>, HttpError> {
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json,text/plain,*/*")
        .call()
        .map_err(|error| match error {
            ureq::Error::Timeout(_) => HttpError::Timeout,
            error => HttpError::Transport(bound_error(error)),
        })?;

    let code = response.status().as_u16();
    if !(200..300).contains(&code) {
        let retry_after = if code == 429 {
            response
                .headers()
                .get("Retry-After")
                .and_then(|header| header.to_str().ok())
                .and_then(|value| parse_retry_after(Some(value)))
        } else {
            None
        };
        return Err(HttpError::Status { code, retry_after });
    }

    let mut body = Vec::new();
    let mut reader = response
        .body_mut()
        .as_reader()
        .take(max_bytes.saturating_add(1));
    reader.read_to_end(&mut body).map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            HttpError::Timeout
        } else {
            HttpError::Read(bound_error(error))
        }
    })?;
    if body.len() as u64 > max_bytes {
        return Err(HttpError::BodyTooLarge { maximum: max_bytes });
    }
    Ok(body)
}

fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    value?
        .parse::<u64>()
        .ok()
        .map(|seconds| Duration::from_secs(seconds.min(RETRY_AFTER_MAX_SECS)))
}

fn bound_error(error: impl fmt::Display) -> String {
    error
        .to_string()
        .chars()
        .take(HTTP_ERROR_MAX_CHARS)
        .collect()
}

pub(crate) const fn http_failure_category(error: &HttpError) -> FailureCategory {
    match error {
        HttpError::InvalidUrl => FailureCategory::Validation,
        HttpError::Timeout => FailureCategory::Timeout,
        HttpError::Transport(_) => FailureCategory::Transport,
        HttpError::Status { .. } => FailureCategory::Http,
        HttpError::Parse(_) => FailureCategory::Parse,
        HttpError::BodyTooLarge { .. } | HttpError::Read(_) => FailureCategory::Response,
    }
}

/// Warframe.market item slugs are lowercase ascii with `_` / `-` only.
pub fn is_safe_market_slug(slug: &str) -> bool {
    let len = slug.len();
    (1..=96).contains(&len)
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use super::{
        HttpError, WORLDSTATE_HOST, get_with_agent, http_agent, http_failure_category,
        is_safe_market_slug, parse_retry_after, validate_https_url,
    };
    use crate::runtime::widget_diagnostics::FailureCategory;

    #[test]
    fn warframe_diagnostic_http_errors_map_to_stable_private_categories() {
        assert_eq!(
            http_failure_category(&HttpError::Transport("private detail".to_owned())),
            FailureCategory::Transport
        );
        assert_eq!(
            http_failure_category(&HttpError::Status {
                code: 503,
                retry_after: None,
            }),
            FailureCategory::Http
        );
        assert_eq!(
            http_failure_category(&HttpError::BodyTooLarge { maximum: 1 }),
            FailureCategory::Response
        );
        assert_eq!(
            http_failure_category(&HttpError::Parse("private detail".to_owned())),
            FailureCategory::Parse
        );
        assert_eq!(
            http_failure_category(&HttpError::InvalidUrl),
            FailureCategory::Validation
        );
    }

    fn serve_response(
        response: &'static [u8],
    ) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = Arc::clone(&calls);
        let join = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        server_calls.fetch_add(1, Ordering::SeqCst);
                        stream
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .unwrap();
                        let mut request = [0_u8; 1_024];
                        let _ = stream.read(&mut request);
                        stream.write_all(response).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("local HTTP server failed: {error}"),
                }
            }
        });
        (format!("http://{address}/first"), calls, join)
    }

    #[test]
    fn https_policy_requires_exact_host_without_credentials_or_custom_port() {
        assert!(validate_https_url("https://api.warframe.com/x", WORLDSTATE_HOST).is_ok());
        assert!(validate_https_url("https://api.warframe.com:443/x", WORLDSTATE_HOST).is_ok());
        assert!(validate_https_url("https://API.WARFRAME.COM/x", WORLDSTATE_HOST).is_ok());
        assert!(validate_https_url("https://api.warframe.com:444/x", WORLDSTATE_HOST).is_err());
        assert!(
            validate_https_url("https://api.warframe.com@evil.example/x", WORLDSTATE_HOST).is_err()
        );
        assert!(
            validate_https_url("https://user:pass@api.warframe.com/x", WORLDSTATE_HOST).is_err()
        );
        assert!(validate_https_url("https://@api.warframe.com/x", WORLDSTATE_HOST).is_err());
        assert!(validate_https_url("HTTPS://@api.warframe.com/x", WORLDSTATE_HOST).is_err());
        assert!(validate_https_url("http://api.warframe.com/x", WORLDSTATE_HOST).is_err());
        assert!(validate_https_url("https://api.warframe.com.evil/x", WORLDSTATE_HOST).is_err());
    }

    #[test]
    fn redirect_is_returned_as_status_without_a_followup_request() {
        let response = b"HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (url, calls, server) = serve_response(response);

        let result = get_with_agent(&http_agent(), &url, 64);

        assert_eq!(
            result,
            Err(HttpError::Status {
                code: 302,
                retry_after: None,
            })
        );
        server.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retry_after_accepts_only_numeric_seconds_and_caps_them() {
        assert_eq!(parse_retry_after(Some("12")), Some(Duration::from_secs(12)));
        assert_eq!(
            parse_retry_after(Some("999")),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after(Some("-1")), None);
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn response_body_limit_has_a_typed_error() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";
        let (url, _, server) = serve_response(response);

        let result = get_with_agent(&http_agent(), &url, 2);

        assert_eq!(result, Err(HttpError::BodyTooLarge { maximum: 2 }));
        server.join().unwrap();
    }

    #[test]
    fn market_slugs_reject_path_injection() {
        assert!(is_safe_market_slug("valkyr_prime_set"));
        assert!(!is_safe_market_slug("../etc/passwd"));
        assert!(!is_safe_market_slug("a/b"));
        assert!(!is_safe_market_slug("HasCaps"));
        assert!(!is_safe_market_slug(""));
        assert!(!is_safe_market_slug(&"x".repeat(100)));
    }
}
