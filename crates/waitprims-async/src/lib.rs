//! Async first-match and poll-cycle runners.
//!
//! Tokio is used for runtime, time, and synchronization only. This crate
//! does not open network sockets.

pub use waitprims_core::{Error, Result};

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn runtime_is_available() {
        tokio::time::sleep(std::time::Duration::from_millis(0)).await;
    }
}
