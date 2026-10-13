use std::path::PathBuf;
use std::time::Duration;

use crate::error::{Error, Result};

/// Where events go and when; `Config::new` takes the defaults of docs/PROTOCOL.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The ingest server; batches go to `{endpoint}/v1/batch`.
    pub endpoint: String,
    /// The source's write key.
    pub write_key: String,
    /// The queue's SQLite file; missing directories are created.
    pub path: PathBuf,
    /// A batch is due once this many events wait.
    pub flush_at: usize,
    /// A batch is due once the oldest event has waited this long.
    pub flush_interval: Duration,
    /// The most events kept: the newest drops the oldest beyond it.
    pub max_queue: usize,
    /// Events older than this are deleted unsent.
    pub max_age: Duration,
    /// The most events in one batch.
    pub max_batch_events: usize,
    /// The most bytes in one batch, counted over the bodies joined by commas.
    pub max_batch_bytes: usize,
}

impl Config {
    pub fn new(
        endpoint: impl Into<String>,
        write_key: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            write_key: write_key.into(),
            path: path.into(),
            flush_at: 20,
            flush_interval: Duration::from_secs(30),
            max_queue: 10_000,
            max_age: Duration::from_secs(30 * 24 * 60 * 60),
            max_batch_events: 100,
            max_batch_bytes: 900_000,
        }
    }

    /// Validates startup settings before the queue is opened and constructs the ingest URL.
    pub(crate) fn batch_url(&self) -> Result<String> {
        if self.flush_at == 0
            || self.flush_interval.is_zero()
            || self.max_queue == 0
            || self.max_age.is_zero()
            || self.max_batch_bytes == 0
        {
            return Err(Error::InvalidConfiguration(
                "queue limits and intervals must be positive",
            ));
        }
        if !(1..=500).contains(&self.max_batch_events) {
            return Err(Error::InvalidConfiguration(
                "a batch must contain between 1 and 500 events",
            ));
        }
        if self.path.as_os_str().is_empty() {
            return Err(Error::InvalidConfiguration(
                "the queue path must not be empty",
            ));
        }
        if self.write_key.is_empty() || !self.write_key.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Error::InvalidConfiguration(
                "a public source write key without whitespace is required",
            ));
        }
        let mut url = url::Url::parse(&self.endpoint).map_err(|_| {
            Error::InvalidConfiguration("the endpoint must be an absolute HTTP(S) URL")
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::InvalidConfiguration(
                "the endpoint must be an HTTP(S) URL without credentials, query or fragment",
            ));
        }
        url.set_path(&format!("{}/v1/batch", url.path().trim_end_matches('/')));
        Ok(url.to_string())
    }
}
