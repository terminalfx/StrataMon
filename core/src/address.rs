//! Turns what a person types into a server's address into a Strata base URL.

use std::net::Ipv6Addr;

/// The port Strata's launcher, run scripts and desktop app all use unless told otherwise.
pub const DEFAULT_PORT: u16 = 8080;

/// Accepts forms like `192.168.1.10`, `strata1:8095`, `strata1.tailnet.ts.net`, `https://host`
/// and bare IPv6 addresses, which is what Tailscale shows. Returns the base URL ending in `/`,
/// or `None` if it can't be a usable address.
pub fn normalize(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }

    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_string(), text),
    };
    if scheme != "http" && scheme != "https" {
        return None;
    }

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (host, port) = split_host(authority)?;
    if !valid_host(&host) {
        return None;
    }

    let port = port.unwrap_or(DEFAULT_PORT);
    let default_for_scheme = if scheme == "https" { 443 } else { 80 };
    Some(if port == default_for_scheme {
        format!("{scheme}://{host}/")
    } else {
        format!("{scheme}://{host}:{port}/")
    })
}

fn split_host(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.parse::<Ipv6Addr>().is_ok() {
        return Some((format!("[{}]", authority.to_ascii_lowercase()), None));
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let (inside, after) = rest.split_once(']')?;
        inside.parse::<Ipv6Addr>().ok()?;
        let host = format!("[{}]", inside.to_ascii_lowercase());
        return match after.strip_prefix(':') {
            Some(port) => Some((host, Some(port.parse().ok()?))),
            None if after.is_empty() => Some((host, None)),
            None => None,
        };
    }

    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_ascii_lowercase(), Some(port.parse().ok()?))),
        None => Some((authority.to_ascii_lowercase(), None)),
    }
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && (host.starts_with('[')
            // ASCII only: a host made of look-alike Unicode letters is a name nobody typed,
            // and the address would display as if it were a real one.
            || host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'))
}
