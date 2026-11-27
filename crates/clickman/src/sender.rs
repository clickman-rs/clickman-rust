use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::backoff;
use crate::batch::{self, Batch, Kind, Urgency};
use crate::clock::{Clock, millis, rfc3339_millis, unix_millis};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::gzip;
use crate::outcome::Outcome;
use crate::store::Store;
use crate::transport::{HttpClient, HttpResponse};

/// A number in [0, 1).
pub(crate) type Random = Arc<dyn Fn() -> f64 + Send + Sync>;

/// Sends batches one at a time. Failures are counted in memory only, so a
/// restarted client sends what is waiting at once.
pub(crate) struct Sender {
    http: Box<dyn HttpClient>,
    url: String,
    identify_url: String,
    config: Config,
    clock: Clock,
    random: Random,
    state: Mutex<SenderState>,
}

#[derive(Default)]
struct SenderState {
    flight: Flight,
    backoff: Backoff,
}

#[derive(Clone, Copy, Default)]
enum Flight {
    #[default]
    Idle,
    InFlight,
}

/// The consecutive failures, and the time in milliseconds before which nothing is sent.
#[derive(Clone, Copy, Default)]
enum Backoff {
    #[default]
    Clear,
    Waiting {
        failures: u32,
        until: i64,
    },
}

/// The one flight; dropping it, done or cancelled, lets the next batch go.
struct FlightGuard<'a>(&'a Mutex<SenderState>);

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        self.0.lock().flight = Flight::Idle;
    }
}

/// What came back for a batch.
struct Answer {
    status: u16,
    retry_after: Option<Duration>,
}

impl Answer {
    const NO_RESPONSE: Self = Self {
        status: 0,
        retry_after: None,
    };

    fn of(response: HttpResponse) -> Self {
        let retry_after = backoff::retry_after(response.retry_after.as_deref());
        Self {
            status: response.status,
            retry_after,
        }
    }
}

impl Sender {
    pub(crate) fn new(
        http: Box<dyn HttpClient>,
        config: Config,
        url: String,
        clock: Clock,
        random: Random,
    ) -> Self {
        let state = Mutex::default();
        let identify_url = Config::identify_url(&url);
        Self {
            http,
            url,
            identify_url,
            config,
            clock,
            random,
            state,
        }
    }

    /// Sends batches until nothing is due or a send fails.
    pub(crate) async fn drain(&self, store: &Store, urgency: Urgency) -> Result<()> {
        loop {
            match self.send_next(store, urgency).await? {
                Some(Outcome::Delivered | Outcome::Refused) => {}
                Some(Outcome::Retry) | None => return Ok(()),
            }
        }
    }

    /// Sends the next batch, or nothing while one is in flight, a retry waits or nothing is due.
    async fn send_next(&self, store: &Store, urgency: Urgency) -> Result<Option<Outcome>> {
        let now = (self.clock)();
        let Some((_flight, may_send)) = self.reserve(unix_millis(now)) else {
            return Ok(None);
        };
        let taken = store.write(|transaction| {
            if !may_send {
                batch::purge(
                    transaction,
                    unix_millis(now).saturating_sub(millis(self.config.max_age)),
                )?;
                return Ok(None);
            }
            batch::take(transaction, &self.config, unix_millis(now), urgency)
        })?;
        let Some(batch) = taken else { return Ok(None) };
        let answer = self.post(&batch, &rfc3339_millis(now)).await?;
        self.settle(store, &batch, &answer).map(Some)
    }

    fn reserve(&self, now: i64) -> Option<(FlightGuard<'_>, bool)> {
        let mut state = self.state.lock();
        match (state.flight, state.backoff) {
            (Flight::InFlight, _) => None,
            (Flight::Idle, backoff) => {
                let may_send = !matches!(backoff, Backoff::Waiting { until, .. } if now < until);
                state.flight = Flight::InFlight;
                Some((FlightGuard(&self.state), may_send))
            }
        }
    }

    /// Events go gzipped to `/v1/batch`; a link goes alone and uncompressed to `/v1/identify`.
    async fn post(&self, batch: &Batch, sent_at: &str) -> Result<Answer> {
        let (url, headers, body) = match batch.kind {
            Kind::Events => {
                let payload = batch.payload(sent_at);
                let body = gzip::compress(payload.as_bytes()).map_err(Error::Compression)?;
                (&self.url, self.headers(true), body)
            }
            Kind::Link => (
                &self.identify_url,
                self.headers(false),
                batch.link_body().into_bytes(),
            ),
        };
        match self.http.post(url, headers, body).await {
            Ok(response) => Ok(Answer::of(response)),
            Err(message) => {
                log::warn!("ClickMan could not reach {url}: {message}");
                Ok(Answer::NO_RESPONSE)
            }
        }
    }

    fn headers(&self, gzipped: bool) -> Vec<(String, String)> {
        let authorization = format!("Bearer {}", self.config.write_key);
        let mut headers = vec![
            ("Authorization".to_owned(), authorization),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ];
        if gzipped {
            headers.push(("Content-Encoding".to_owned(), "gzip".to_owned()));
        }
        headers
    }

    fn settle(&self, store: &Store, batch: &Batch, answer: &Answer) -> Result<Outcome> {
        let status = answer.status;
        let (outcome, what) = match batch.kind {
            Kind::Events => (
                Outcome::of(status),
                format!("{} events", batch.bodies.len()),
            ),
            Kind::Link => (Outcome::of_link(status), "a visitor link".to_owned()),
        };
        match outcome {
            Outcome::Delivered => self.leave_queue(store, batch)?,
            Outcome::Refused => {
                log::warn!("ClickMan dropped {what} refused with status {status}");
                self.leave_queue(store, batch)?;
            }
            Outcome::Retry => self.back_off(answer, &what),
        }
        Ok(outcome)
    }

    /// The batch leaves the queue, which ends any run of failures.
    fn leave_queue(&self, store: &Store, batch: &Batch) -> Result<()> {
        store.write(|transaction| batch::remove(transaction, batch))?;
        self.state.lock().backoff = Backoff::Clear;
        Ok(())
    }

    fn back_off(&self, answer: &Answer, what: &str) {
        let mut state = self.state.lock();
        let failures = match state.backoff {
            Backoff::Clear => 1,
            Backoff::Waiting { failures, .. } => failures.saturating_add(1),
        };
        let delay = backoff::delay(failures, answer.retry_after, (self.random)());
        let until = unix_millis((self.clock)()).saturating_add(millis(delay));
        state.backoff = Backoff::Waiting { failures, until };
        drop(state);
        let status = answer.status;
        log::info!("ClickMan retries {what} in {delay:?} after status {status}");
    }
}
