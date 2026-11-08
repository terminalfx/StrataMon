//! Polls each server on its own thread, so a dead one never delays the others.

use crate::transport::UreqTransport;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use stratamon_core::client::{PollResult, StrataClient};
use stratamon_core::config::{AppConfig, ServerConfig};
use stratamon_core::settings::describe_connection;

/// Wakes sleeping threads straight away when it's time to stop.
#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl Stop {
    fn signal(&self) {
        if let Ok(mut stopped) = self.stopped.lock() {
            *stopped = true;
        }
        self.wake.notify_all();
    }

    /// Sleeps, and returns true if it's time to stop.
    fn wait(&self, duration: Duration) -> bool {
        let Ok(stopped) = self.stopped.lock() else {
            return true;
        };

        match self.wake.wait_timeout_while(stopped, duration, |stopped| !*stopped) {
            Ok((stopped, _)) => *stopped,
            Err(_) => true,
        }
    }
}

pub struct Monitor {
    stop: Arc<Stop>,
}

impl Monitor {
    pub fn start(config: &AppConfig, report: impl Fn(usize, PollResult) -> bool + Clone + Send + 'static) -> Self {
        let stop = Arc::new(Stop::default());
        let period = Duration::from_secs(u64::from(config.poll_seconds.max(1)));

        for (index, server) in config.servers.iter().enumerate() {
            let (server, stop, report) = (server.clone(), stop.clone(), report.clone());
            std::thread::spawn(move || watch(index, &server, period, &stop, report));
        }

        Self { stop }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.signal();
    }
}

fn watch(index: usize, server: &ServerConfig, period: Duration, stop: &Stop, report: impl Fn(usize, PollResult) -> bool) {
    let client = StrataClient::new(UreqTransport::new());
    let mut working_address: Option<String> = None;

    loop {
        let started = Instant::now();
        let result = client.poll(server, working_address.as_deref());
        if result.is_success() {
            working_address.clone_from(&result.address);
        }

        // Stops once the window has gone, or settings changed and a new monitor took over.
        if !report(index, result) || stop.wait(period.saturating_sub(started.elapsed())) {
            return;
        }
    }
}

/// Each address on its own: during setup you want to know both routes work, not just that one does.
pub fn test_connection(server: &ServerConfig) -> String {
    std::thread::scope(|scope| {
        let attempts: Vec<_> = server
            .addresses
            .iter()
            .map(|address| {
                scope.spawn(move || {
                    let one = ServerConfig { addresses: vec![address.clone()], ..server.clone() };
                    let result = StrataClient::new(UreqTransport::new()).poll(&one, None);
                    match (&result.snapshot, &result.detail) {
                        (Some(snapshot), _) => format!("{address}: {}", describe_connection(snapshot)),
                        (None, detail) => format!("{address}: {}", detail.as_deref().unwrap_or("")),
                    }
                })
            })
            .collect();

        attempts.into_iter().filter_map(|attempt| attempt.join().ok()).collect::<Vec<_>>().join("\n")
    })
}
