//! The real network, behind the same trait the tests fake.

use std::io::{ErrorKind, Read};
use stratamon_core::client::{CONNECT_TIMEOUT, MAX_RESPONSE_BYTES, NetError, Reply, Request, Transport};
use ureq::Agent;
use ureq::config::Config;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub struct UreqTransport {
    agent: Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        // Servers are on the LAN or the tailnet, so a system proxy can only get in the way,
        // and a redirect means a captive portal or some other device, not Strata.
        let config = Config::builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .tls_config(tls())
            .build();
        Self { agent: config.into() }
    }
}

/// Windows' own TLS with Windows' own certificates. This has to be set explicitly: left at the
/// default, the library looks for a certificate list that isn't built in, and panics.
#[cfg(windows)]
pub fn tls() -> TlsConfig {
    TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build()
}

#[cfg(not(windows))]
pub fn tls() -> TlsConfig {
    TlsConfig::builder().provider(TlsProvider::Rustls).root_certs(RootCerts::WebPki).build()
}

impl Transport for UreqTransport {
    fn get(&self, request: &Request) -> Result<Reply, NetError> {
        let mut call = self.agent.get(&request.url).config().timeout_global(Some(request.timeout)).build().header("Accept", "application/json");
        if let Some(key) = &request.bearer {
            call = call.header("Authorization", format!("Bearer {key}"));
        }

        let mut response = call.call().map_err(net_error)?;
        let status = response.status().as_u16();

        // Only a little over the limit is read, enough for the client to see it's too big. A read that
        // stops early is a network failure, not a short-but-complete answer, so report it as one.
        let mut body = Vec::new();
        let limit = MAX_RESPONSE_BYTES as u64 + 1;
        let read = response.body_mut().with_config().limit(limit).reader().take(limit).read_to_end(&mut body);
        match read {
            Ok(_) => Ok(Reply { status, body }),
            // Only a 200's body is ever looked at. Any other status is the whole answer, and
            // calling it a network failure would hide a definite one, such as a rejected key.
            Err(_) if status != 200 => Ok(Reply { status, body }),
            Err(error) => Err(body_error(error)),
        }
    }
}

fn body_error(error: std::io::Error) -> NetError {
    match error.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => return NetError::TimedOut,
        ErrorKind::ConnectionRefused => return NetError::Refused,
        _ => {}
    }

    // While reading a body, ureq hands its own errors back wrapped in an io::Error of kind
    // Other, its timeout among them, so look inside before settling on a message.
    let message = error.to_string();
    let found = match error.into_inner().map(|inner| inner.downcast::<ureq::Error>()) {
        Some(Ok(inner)) => net_error(*inner),
        _ => NetError::Other(message),
    };
    match found {
        NetError::Other(message) => NetError::Other(format!("The connection stopped in the middle of an answer. {message}")),
        known => known,
    }
}

fn net_error(error: ureq::Error) -> NetError {
    match error {
        ureq::Error::Timeout(_) => NetError::TimedOut,
        ureq::Error::Io(io) => match io.kind() {
            ErrorKind::ConnectionRefused => NetError::Refused,
            ErrorKind::TimedOut | ErrorKind::WouldBlock => NetError::TimedOut,
            _ => NetError::Other(io.to_string()),
        },
        ureq::Error::HostNotFound => NetError::Other("That name wasn't found on the network.".to_string()),
        other => NetError::Other(other.to_string()),
    }
}
