use std::cell::RefCell;
use stratamon_core::address::normalize;
use stratamon_core::client::*;
use stratamon_core::config::ServerConfig;
use stratamon_core::models::TelemetryAccess;

const LAN: &str = "192.168.1.10";
const TAILSCALE: &str = "100.93.1.2";
const KEY: &str = "sk-strata-secret";

// A real /health response, with no key needed.
const HEALTH_BODY: &str = r#"{"status": "ok", "max_context": 32768, "model": "qwen", "images": false, "api_key": true, "image_generation": false, "loaded": true, "service": "strata"}"#;
const WORKING_TELEMETRY: &str = include_str!("samples/working-telemetry.json");
const WORKING_METRICS: &str = include_str!("samples/working-metrics.json");

type Answer = Result<Reply, NetError>;

fn server() -> ServerConfig {
    ServerConfig { name: "strata-1".into(), addresses: vec![LAN.into(), TAILSCALE.into()], key: KEY.into(), ..Default::default() }
}

/// Answers by host and path, and records every request so tests can inspect what was sent.
struct FakeNetwork<F> {
    respond: F,
    requests: RefCell<Vec<Request>>,
}

impl<F: Fn(&str, &str) -> Answer> Transport for FakeNetwork<F> {
    fn get(&self, request: &Request) -> Answer {
        self.requests.borrow_mut().push(request.clone());
        let (host, path) = split(&request.url);
        (self.respond)(&host, &path)
    }
}

fn split(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    (authority.rsplit_once(':').map_or(authority, |(host, _)| host).to_string(), path.to_string())
}

impl<F> FakeNetwork<F> {
    fn to(&self, host: &str) -> Vec<Request> {
        self.requests.borrow().iter().filter(|r| split(&r.url).0 == host).cloned().collect()
    }

    fn all(&self) -> Vec<Request> {
        self.requests.borrow().clone()
    }
}

fn json(status: u16, body: &str) -> Answer {
    Ok(Reply { status, body: body.as_bytes().to_vec() })
}

fn empty(status: u16) -> Answer {
    json(status, "{}")
}

fn strata(path: &str) -> Answer {
    match path {
        HEALTH_PATH => json(200, HEALTH_BODY),
        TELEMETRY_PATH => json(200, WORKING_TELEMETRY),
        METRICS_PATH => json(200, WORKING_METRICS),
        _ => empty(404),
    }
}

fn unreachable_host() -> Answer {
    Err(NetError::Other("No route to host".into()))
}

fn poll<F: Fn(&str, &str) -> Answer>(respond: F, preferred: Option<&str>, server: ServerConfig) -> (PollResult, StrataClient<FakeNetwork<F>>) {
    let client = StrataClient::new(FakeNetwork { respond, requests: RefCell::new(Vec::new()) });
    let result = client.poll(&server, preferred);
    (result, client)
}

#[test]
fn uses_the_first_address_when_it_answers() {
    let (result, _) = poll(|_, path| strata(path), None, server());
    assert!(result.is_success());
    assert_eq!(result.address.as_deref(), Some(LAN));
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::Full);
}

#[test]
fn falls_back_to_the_next_address() {
    let (result, _) = poll(|host, path| if host == LAN { unreachable_host() } else { strata(path) }, None, server());
    assert!(result.is_success());
    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
}

#[test]
fn tries_the_last_working_address_first() {
    let (result, client) = poll(|_, path| strata(path), Some(TAILSCALE), server());
    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
    assert!(client.transport().to(LAN).is_empty());
}

#[test]
fn a_laptop_waking_on_other_wifi_does_not_send_the_key_to_whatever_has_that_address_now() {
    let (result, client) = poll(
        |host, path| if host == LAN { json(200, "<html>cafe wifi</html>") } else { strata(path) },
        Some(LAN),
        server(),
    );

    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
    assert!(client.transport().to(LAN).iter().all(|r| r.bearer.is_none()));
}

#[test]
fn goes_back_to_the_lan_when_the_preferred_address_stops_working() {
    let (result, _) = poll(|host, path| if host == TAILSCALE { unreachable_host() } else { strata(path) }, Some(TAILSCALE), server());
    assert_eq!(result.address.as_deref(), Some(LAN));
}

#[test]
fn sends_the_key_as_a_bearer_token_to_the_monitoring_endpoints() {
    let (_, client) = poll(|_, path| strata(path), None, server());

    for request in client.transport().all().iter().filter(|r| split(&r.url).1 != HEALTH_PATH) {
        assert_eq!(request.bearer.as_deref(), Some(KEY));
    }
}

#[test]
fn never_sends_the_key_to_the_address_check() {
    let (_, client) = poll(|_, path| strata(path), None, server());
    let requests = client.transport().all();
    let health = requests.iter().find(|r| split(&r.url).1 == HEALTH_PATH).unwrap();
    assert!(health.bearer.is_none());
}

#[test]
fn a_device_that_is_not_strata_never_receives_the_key() {
    for (status, body) in [
        (404, "<html>router page</html>"),
        (200, "<html>router page</html>"),
        (200, r#"{"status":"ok"}"#),
        (200, r#"{"service":"something else"}"#),
        (200, r#"["service","strata"]"#),
    ] {
        let only_lan = ServerConfig { addresses: vec![LAN.into()], ..server() };
        let (result, client) = poll(|_, _| json(status, body), None, only_lan);

        assert_eq!(result.failure, PollFailure::NotStrata, "{status} {body}");
        assert!(client.transport().to(LAN).iter().all(|r| r.bearer.is_none()));
    }
}

#[test]
fn moves_past_a_device_that_is_not_strata_to_the_real_server() {
    let (result, client) = poll(|host, path| if host == LAN { json(200, "<html>cafe wifi</html>") } else { strata(path) }, None, server());

    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
    assert!(client.transport().to(LAN).iter().all(|r| r.bearer.is_none()));
}

#[test]
fn a_read_only_key_still_works_with_what_it_can_see() {
    let (result, _) = poll(|_, path| if path == METRICS_PATH { empty(401) } else { strata(path) }, None, server());

    assert!(result.is_success());
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::TelemetryOnly);
}

#[test]
fn an_older_strata_without_telemetry_still_works_from_metrics() {
    let (result, _) = poll(|_, path| if path == TELEMETRY_PATH { empty(404) } else { strata(path) }, None, server());

    assert!(result.is_success());
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::MetricsOnly);
}

#[test]
fn a_wrong_key_says_so() {
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { empty(401) }, None, server());

    assert!(!result.is_success());
    assert_eq!(result.failure, PollFailure::KeyRejected);
    assert!(result.detail.unwrap().contains("rejected the key"));
}

#[test]
fn a_missing_key_asks_for_one() {
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { empty(401) }, None, ServerConfig { key: String::new(), ..server() });

    assert_eq!(result.failure, PollFailure::KeyMissing);
    assert!(result.detail.unwrap().contains("needs a key"));
}

#[test]
fn no_key_means_no_authorization() {
    let (_, client) = poll(|_, path| strata(path), None, ServerConfig { key: String::new(), ..server() });
    assert!(client.transport().all().iter().all(|r| r.bearer.is_none()));
}

#[test]
fn a_refused_host_name_explains_how_to_fix_it() {
    let (result, _) = poll(|_, _| empty(403), None, server());

    assert_eq!(result.failure, PollFailure::HostRefused);
    assert!(result.detail.unwrap().contains("allowed_hosts"));
}

#[test]
fn a_strata_with_no_monitoring_at_all_asks_to_update() {
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { empty(404) }, None, server());

    assert_eq!(result.failure, PollFailure::NoTelemetry);
    assert!(result.detail.unwrap().contains("Update"));
}

#[test]
fn a_new_telemetry_format_is_refused_rather_than_misread() {
    let (result, _) = poll(|_, path| if path == TELEMETRY_PATH { json(200, r#"{"schema_version":2}"#) } else { strata(path) }, None, server());

    assert!(!result.is_success());
    assert_eq!(result.failure, PollFailure::Incompatible);
    assert!(result.detail.unwrap().contains("Update"));
}

#[test]
fn a_malformed_format_number_is_refused_without_crashing() {
    for body in [r#"{"schema_version":1.5}"#, r#"{"schema_version":99999999999}"#, r#"{"schema_version":"1"}"#, "{}"] {
        let (result, _) = poll(|_, path| if path == TELEMETRY_PATH { json(200, body) } else { strata(path) }, None, server());
        assert_eq!(result.failure, PollFailure::Incompatible, "{body}");
    }
}

#[test]
fn a_closed_port_is_told_apart_from_a_machine_that_is_off() {
    let (result, _) = poll(|_, _| Err(NetError::Refused), None, server());

    assert_eq!(result.failure, PollFailure::NotListening);
    assert!(result.detail.unwrap().contains("nothing is listening"));
}

#[test]
fn a_proxy_with_nothing_behind_it_is_a_server_error() {
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { empty(502) }, None, server());

    assert_eq!(result.failure, PollFailure::ServerError);
    assert!(result.detail.unwrap().contains("502"));
}

#[test]
fn metrics_from_something_other_than_strata_are_ignored() {
    let (result, _) = poll(|_, path| if path == METRICS_PATH { json(200, "{}") } else { strata(path) }, None, server());
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::TelemetryOnly);
}

#[test]
fn an_oversized_response_is_not_read() {
    let huge = format!("{}{{}}", " ".repeat(MAX_RESPONSE_BYTES + 1));
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { json(200, &huge) }, None, server());
    assert!(!result.is_success());
}

#[test]
fn deeply_nested_json_is_rejected_rather_than_overflowing_the_stack() {
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    let (result, _) = poll(|_, path| if path == HEALTH_PATH { strata(path) } else { json(200, &deep) }, None, server());
    assert!(!result.is_success());
}

#[test]
fn reports_a_rejection_rather_than_the_other_address_timing_out() {
    let (result, _) = poll(
        |host, path| {
            if host == LAN {
                unreachable_host()
            } else if path == HEALTH_PATH {
                strata(path)
            } else {
                empty(401)
            }
        },
        None,
        server(),
    );

    assert_eq!(result.failure, PollFailure::KeyRejected);
    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
}

#[test]
fn a_server_with_no_addresses_fails_cleanly() {
    let (result, _) = poll(|_, path| strata(path), None, ServerConfig { addresses: vec![], ..server() });
    assert_eq!(result.failure, PollFailure::Unreachable);
}

#[test]
fn a_bad_address_does_not_stop_the_good_one_from_working() {
    let (result, _) = poll(|_, path| strata(path), None, ServerConfig { addresses: vec!["not a host".into(), TAILSCALE.into()], ..server() });
    assert!(result.is_success());
}

#[test]
fn a_silent_address_times_out_and_moves_on() {
    let (result, _) = poll(|host, path| if host == LAN { Err(NetError::TimedOut) } else { strata(path) }, None, server());
    assert!(result.is_success());
    assert_eq!(result.address.as_deref(), Some(TAILSCALE));
}

// A busy server can be slow on /metrics alone. Throwing away the telemetry that did arrive
// would call it unreachable, and three polls like that would call it offline.
#[test]
fn one_slow_path_does_not_waste_the_answer_from_the_other() {
    let (result, _) = poll(|_, path| if path == METRICS_PATH { Err(NetError::TimedOut) } else { strata(path) }, None, server());
    assert!(result.is_success());
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::TelemetryOnly);

    let (result, _) = poll(|_, path| if path == TELEMETRY_PATH { Err(NetError::TimedOut) } else { strata(path) }, None, server());
    assert!(result.is_success());
    assert_eq!(result.snapshot.unwrap().access, TelemetryAccess::MetricsOnly);
}

// With a read-only key /metrics always says 401. That isn't a rejected key just because
// /v1/telemetry didn't get through this once.
#[test]
fn half_an_answer_without_readings_is_reported_as_the_network_failure() {
    let (result, _) = poll(
        |_, path| match path {
            TELEMETRY_PATH => Err(NetError::TimedOut),
            METRICS_PATH => empty(401),
            _ => strata(path),
        },
        None,
        server(),
    );
    assert_eq!(result.failure, PollFailure::Unreachable);
    assert_eq!(result.detail.as_deref(), Some("Timed out."));
}

#[test]
fn a_timeout_is_reported_as_one() {
    let (result, _) = poll(|_, _| Err(NetError::TimedOut), None, server());
    assert_eq!(result.failure, PollFailure::Unreachable);
    assert_eq!(result.detail.as_deref(), Some("Timed out."));
}

#[test]
fn every_request_gets_the_time_left_in_the_attempt() {
    let (_, client) = poll(|_, path| strata(path), None, server());
    assert!(client.transport().all().iter().all(|r| r.timeout <= ATTEMPT_TIMEOUT && !r.timeout.is_zero()));
}

#[test]
fn the_connect_limit_outlasts_a_windows_refusal() {
    assert!(CONNECT_TIMEOUT.as_millis() > 2100, "Windows takes about 2.05 s to report a closed port");
    assert!(ATTEMPT_TIMEOUT > CONNECT_TIMEOUT);
}

#[test]
fn normalizes_what_people_type() {
    for (input, expected) in [
        ("192.168.1.10", "http://192.168.1.10:8080/"),
        ("  192.168.1.10  ", "http://192.168.1.10:8080/"),
        ("192.168.1.10:8095", "http://192.168.1.10:8095/"),
        ("strata1", "http://strata1:8080/"),
        ("strata1.tail1234.ts.net", "http://strata1.tail1234.ts.net:8080/"),
        ("100.101.102.103", "http://100.101.102.103:8080/"),
        ("fd7a:115c:a1e0::1", "http://[fd7a:115c:a1e0::1]:8080/"),
        ("[fd7a:115c:a1e0::1]:9000", "http://[fd7a:115c:a1e0::1]:9000/"),
        ("http://strata1", "http://strata1:8080/"),
        ("http://strata1:80", "http://strata1/"),
        ("https://strata1", "https://strata1:8080/"),
        ("https://strata1:443/", "https://strata1/"),
        ("http://strata1:9000/some/path?x=1", "http://strata1:9000/"),
        ("NEDM2", "http://nedm2:8080/"),
    ] {
        assert_eq!(normalize(input).as_deref(), Some(expected), "{input}");
    }
}

#[test]
fn refuses_what_cant_be_an_address() {
    for input in ["", "   ", "ftp://strata1", "http://", "not a host", "host:notaport", "[not-ipv6]:80"] {
        assert_eq!(normalize(input), None, "{input:?}");
    }
}
