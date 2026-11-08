//! Reads what a Strata server reports about itself, trying each of its addresses until one works.

use crate::address;
use crate::config::ServerConfig;
use crate::models::StrataSnapshot;
use crate::parser;
use serde_json::Value;
use std::time::{Duration, Instant};

pub const HEALTH_PATH: &str = "health";
pub const TELEMETRY_PATH: &str = "v1/telemetry";
pub const METRICS_PATH: &str = "metrics";
pub const TELEMETRY_SCHEMA: i64 = 1;

/// The same limit Strata's own monitor applies, since the data comes from the network.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Windows takes just over 2 seconds to report a closed port, and it has to finish, because a
/// refusal means the PC is on and only Strata has stopped. A timeout can't tell you that.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(2500);
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);

const OK: u16 = 200;
const UNAUTHORIZED: u16 = 401;
const FORBIDDEN: u16 = 403;
const NOT_FOUND: u16 = 404;

/// Ordered from least to most informative. See [`StrataClient::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum PollFailure {
    #[default]
    None,
    Unreachable,
    NotListening,
    NotStrata,
    ServerError,
    NoTelemetry,
    Incompatible,
    HostRefused,
    KeyMissing,
    KeyRejected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PollResult {
    pub snapshot: Option<StrataSnapshot>,
    pub failure: PollFailure,
    pub address: Option<String>,
    pub detail: Option<String>,
}

impl PollResult {
    pub fn is_success(&self) -> bool {
        self.snapshot.is_some()
    }
}

/// One GET. There's deliberately no other kind of request, so this can't change a server.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub bearer: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// The connection was refused, so the PC is on but nothing is listening.
    Refused,
    TimedOut,
    Other(String),
}

/// One GET against a Strata URL. The body must arrive capped at [`MAX_RESPONSE_BYTES`] plus a
/// byte: the size check below throws the whole thing away rather than parse it, so an
/// uncapped implementation would let a broken or hostile server fill this process's memory.
pub trait Transport {
    fn get(&self, request: &Request) -> Result<Reply, NetError>;
}

pub struct StrataClient<T> {
    transport: T,
    attempt_timeout: Duration,
}

impl<T: Transport> StrataClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, attempt_timeout: ATTEMPT_TIMEOUT }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// `preferred` is the address that last worked. It's tried first, so a dead LAN address
    /// doesn't add a timeout to every poll while you're on Tailscale.
    pub fn poll(&self, server: &ServerConfig, preferred: Option<&str>) -> PollResult {
        if server.addresses.is_empty() {
            return PollResult {
                snapshot: None,
                failure: PollFailure::Unreachable,
                address: None,
                detail: Some("No address is configured.".to_string()),
            };
        }

        let mut addresses: Vec<&String> = server.addresses.iter().collect();
        addresses.sort_by_key(|address| Some(address.as_str()) != preferred);

        let mut most_useful: Option<PollResult> = None;
        for address in addresses {
            let result = self.try_address(server, address);
            if result.is_success() {
                return result;
            }

            // Strata rejecting the key says more about the problem than
            // the other address timing out, so report that one.
            if most_useful.as_ref().is_none_or(|best| result.failure > best.failure) {
                most_useful = Some(result);
            }
        }

        most_useful.unwrap_or_else(|| failure(PollFailure::Unreachable, "", "No address is configured."))
    }

    fn try_address(&self, server: &ServerConfig, address: &str) -> PollResult {
        let Some(base) = address::normalize(address) else {
            return failure(PollFailure::Unreachable, address, "This isn't a valid address.");
        };

        match self.read_server(server, address, &base) {
            Ok(result) => result,
            Err(NetError::TimedOut) => failure(PollFailure::Unreachable, address, "Timed out."),
            Err(NetError::Refused) => failure(
                PollFailure::NotListening,
                address,
                "The PC answered, but nothing is listening on this port. Strata may be stopped, or set to this PC only.",
            ),
            Err(NetError::Other(message)) => failure(PollFailure::Unreachable, address, &message),
        }
    }

    fn read_server(&self, server: &ServerConfig, address: &str, base: &str) -> Result<PollResult, NetError> {
        let started = Instant::now();

        // Every poll, even for an address that just worked: a laptop that sleeps at home and
        // wakes on other Wi-Fi may find a different device at that same LAN address.
        if let Some(problem) = self.identify(address, base, started)? {
            return Ok(problem);
        }

        let key = Some(server.key.as_str()).filter(|key| !key.is_empty());
        let telemetry = self.fetch(base, TELEMETRY_PATH, key, started);
        let metrics = self.fetch(base, METRICS_PATH, key, started);

        // A server that answers on only one of the two paths is a shape interpret supports,
        // so one path failing on the network mustn't throw away readings the other already
        // gave. Anything short of readings is reported as the network failure, though: half
        // an answer can't prove more. A 401 from /metrics is normal for a read-only key, and
        // isn't a rejected key just because /v1/telemetry didn't get through this time.
        let partly = |result: PollResult, error: NetError| if result.is_success() { Ok(result) } else { Err(error) };
        match (telemetry, metrics) {
            (Ok(telemetry), Ok(metrics)) => Ok(interpret(server, address, &telemetry, &metrics)),
            (Ok(telemetry), Err(error)) => partly(interpret(server, address, &telemetry, &Answer::NONE), error),
            (Err(error), Ok(metrics)) => partly(interpret(server, address, &Answer::NONE, &metrics), error),
            (Err(error), Err(_)) => Err(error),
        }
    }

    /// Asks an address what it is before sending it the key. /health needs no key, so a device
    /// that merely happens to answer at a LAN address on some other network never receives it.
    fn identify(&self, address: &str, base: &str, started: Instant) -> Result<Option<PollResult>, NetError> {
        let health = self.fetch(base, HEALTH_PATH, None, started)?;
        if health.status == FORBIDDEN {
            return Ok(Some(host_refused(address)));
        }

        let is_strata = health.status == OK
            && health.json.as_ref().and_then(|root| root.get("service")).and_then(Value::as_str) == Some("strata");

        Ok(if is_strata {
            None
        } else {
            Some(failure(PollFailure::NotStrata, address, "Something answered here, but it isn't Strata."))
        })
    }

    fn fetch(&self, base: &str, path: &str, key: Option<&str>, started: Instant) -> Result<Answer, NetError> {
        let timeout = self.attempt_timeout.checked_sub(started.elapsed()).ok_or(NetError::TimedOut)?;
        let request = Request { url: format!("{base}{path}"), bearer: key.map(String::from), timeout };
        let reply = self.transport.get(&request)?;
        let too_big = reply.body.len() > MAX_RESPONSE_BYTES;
        Ok(Answer { status: reply.status, json: if reply.status == OK && !too_big { read_json(&reply.body) } else { None }, too_big })
    }
}

struct Answer {
    status: u16,

    /// Only for a 200 whose body is a JSON object within the size limit.
    json: Option<Value>,

    /// The body was longer than everything StrataMon reads, so it was never looked at.
    too_big: bool,
}

impl Answer {
    /// Stands in for a path whose request failed on the network, so nothing came back at all.
    const NONE: Answer = Answer { status: 0, json: None, too_big: false };
}

fn read_json(body: &[u8]) -> Option<Value> {
    if body.len() > MAX_RESPONSE_BYTES {
        return None;
    }

    serde_json::from_slice(body).ok().filter(Value::is_object)
}

fn interpret(server: &ServerConfig, address: &str, telemetry: &Answer, metrics: &Answer) -> PollResult {
    let mut telemetry_data = None;
    if telemetry.status == OK {
        // An unreadable telemetry body is not a dead end. /metrics beside it may still carry
        // everything, and a server that answers on only one of the two paths is a supported
        // shape. The checks below decide what to report when neither one yields data.
        if let Some(root) = &telemetry.json {
            // 1.0 is a plausible spelling of 1 from any JSON writer, and as_i64 says no to it.
            // Fractions stay nonsense: they read as no format at all, refusing the poll.
            let schema = root
                .get("schema_version")
                .and_then(Value::as_f64)
                .filter(|value| value.fract() == 0.0 && (0.0..=f64::from(i32::MAX)).contains(value))
                .map(|value| value as i64)
                .unwrap_or(0);
            if schema != TELEMETRY_SCHEMA {
                return failure(
                    PollFailure::Incompatible,
                    address,
                    &format!("This Strata reports telemetry format {schema}, but StrataMon reads format {TELEMETRY_SCHEMA}. Update whichever is older."),
                );
            }

            telemetry_data = Some(root);
        }
    }

    // The Monitor's test for a usable report: an old server, or something else, won't have "live".
    let metrics_data = metrics.json.as_ref().filter(|report| report.get("live").is_some_and(Value::is_object));

    if telemetry_data.is_some() || metrics_data.is_some() {
        return PollResult {
            snapshot: Some(parser::parse(metrics_data, telemetry_data)),
            failure: PollFailure::None,
            address: Some(address.to_string()),
            detail: None,
        };
    }

    if telemetry.status == UNAUTHORIZED || metrics.status == UNAUTHORIZED {
        return if server.key.is_empty() {
            failure(PollFailure::KeyMissing, address, "This Strata needs a key. Add its API key or telemetry key in Settings.")
        } else {
            failure(PollFailure::KeyRejected, address, "Strata rejected the key.")
        };
    }

    if telemetry.status == FORBIDDEN || metrics.status == FORBIDDEN {
        return host_refused(address);
    }

    if telemetry.status == NOT_FOUND && metrics.status == NOT_FOUND {
        return failure(PollFailure::NoTelemetry, address, "This Strata has no monitoring endpoints. Update Strata.");
    }

    let worst = telemetry.status.max(metrics.status);
    if worst >= 500 {
        failure(PollFailure::ServerError, address, &format!("Strata couldn't answer (HTTP {worst})."))
    } else if telemetry.too_big || metrics.too_big {
        // A response was thrown away unread. "wasn't Strata telemetry" would blame the shape
        // of an answer that was never looked at.
        failure(PollFailure::NotStrata, address, &format!("A response was longer than the {} MB that StrataMon reads.", MAX_RESPONSE_BYTES / (1024 * 1024)))
    } else {
        failure(PollFailure::NotStrata, address, &format!("The response wasn't Strata telemetry (HTTP {worst})."))
    }
}

fn host_refused(address: &str) -> PollResult {
    failure(
        PollFailure::HostRefused,
        address,
        "Strata refused this name. Use its IP address, add the name to \"allowed_hosts\", or set an API key.",
    )
}

fn failure(failure: PollFailure, address: &str, detail: &str) -> PollResult {
    PollResult { snapshot: None, failure, address: Some(address.to_string()), detail: Some(detail.to_string()) }
}
