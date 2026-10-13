# ClickMan for Rust

ClickMan records analytics events in a local SQLite queue and sends them to a
ClickMan server in gzip batches. Events survive restarts, and failed requests
keep their batch for a later retry.

The library works with your application's HTTP client and async runtime. You
provide an `HttpClient` implementation and schedule delivery; ClickMan does not
start a runtime or a background timer.

## Add the library

The repository pins its development toolchain in [rust-toolchain.toml](rust-toolchain.toml).
The crate declares Rust 1.94 as its minimum supported version.

For a local checkout, add these dependencies to your application's `Cargo.toml`:

```toml
[dependencies]
clickman = { path = "../clickman-rust/crates/clickman" }
serde_json = "1"
```

## Record and send events

```rust
use clickman::{ClickMan, Config, HttpClient};
use serde_json::json;

async fn example(http: impl HttpClient + 'static) -> clickman::Result<()> {
    let analytics = ClickMan::open(
        Config::new(
            "https://analytics.example.com",
            "your-project-write-key",
            "data/clickman/queue.sqlite",
        ),
        http,
    )?;

    analytics.identify("user_42")?;
    analytics.set_traits(json!({"plan": "pro"}))?;
    analytics.track("page_viewed", json!({"screen": "home"}))?;
    analytics.flush().await?;
    Ok(())
}
```

`track` writes to SQLite without waiting for a network request. Keep the client
alive and call `send_due().await` periodically, for example every five seconds
from your existing runtime. Call `flush().await` when you want to send everything
waiting. Neither call bypasses a failed batch's retry delay.

`HttpClient::post` receives the target URL, headers, and compressed body. Return
the HTTP status and `Retry-After`, or an error if no response arrives. The
[integration-test worker](crates/clickman-e2e-worker/src/main.rs) contains a
working reqwest adapter.

## Configuration and identity

By default, a batch becomes due after 20 queued events or 30 seconds. The queue
holds at most 10,000 events and removes events older than 30 days. Adjust these
limits on `Config` before opening the client.

Use the server's base URL as `endpoint`: the client appends `/v1/batch`. A server
mounted at `/analytics` needs `https://example.com/analytics` as its endpoint.

`identify` sets the actor for later events; `set_traits` merges persistent user
traits. Call `reset` after sign-out. The host supplies context through
`set_context` and records app launches with `app_launched`.

## Develop and test

Run from the repository root:

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/queue-schema.py --check
```

The [protocol](docs/PROTOCOL.md) describes the wire format, queue, identity, and
retry rules. The [local protocol snapshot](protocol/README.md) includes shared
fixtures, so the workspace builds and tests without a server checkout. The
Swift and Kotlin clients implement the same contract.

## License

[MIT](LICENSE).
