use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The server's answer to a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// The `Retry-After` header as it came, if any.
    pub retry_after: Option<String>,
}

/// The host's HTTP stack. An `Err` means no response came back, which counts as
/// status 0: the batch stays for a retry.
///
/// Implementations must apply a finite timeout and return redirect responses
/// without following them. A redirect is retried at the configured endpoint,
/// never forwarded to another URL.
pub trait HttpClient: Send + Sync {
    fn post<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<HttpResponse, String>>;
}
