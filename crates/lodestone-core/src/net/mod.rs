pub mod allowlist;
pub mod client;

pub use allowlist::HostAllowlist;
pub use client::HttpsClient;

use crate::error::NetError;

pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str)
    -> impl std::future::Future<Output = Result<Vec<u8>, NetError>> + Send;
    fn download(
        &self,
        url: &str,
        dest: &std::path::Path,
    ) -> impl std::future::Future<Output = Result<u64, NetError>> + Send;
}

pub fn ensure_https(url: &str) -> Result<(), NetError> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(NetError::DisallowedHost(url.to_string()))
    }
}
