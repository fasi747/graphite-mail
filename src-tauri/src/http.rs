//! One shared web client with time limits, so a stalled connection can never freeze syncing for good.

use std::sync::OnceLock;
use std::time::Duration;

pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(90))
            .build()
            .expect("web client")
    })
}
