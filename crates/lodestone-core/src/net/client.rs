use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Url;
use reqwest::redirect::Policy;

use crate::error::NetError;
use crate::net::allowlist::HostAllowlist;
use crate::net::{HttpClient, ensure_https};

const MAX_TRANSFER_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_REDIRECTS: usize = 6;
const OVERALL_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = "Lodestone/0.1";

pub struct HttpsClient {
    inner: reqwest::Client,
    allowlist: Arc<HostAllowlist>,
}

impl HttpsClient {
    pub fn new(allowlist: HostAllowlist) -> Result<Self, NetError> {
        let allowlist = Arc::new(allowlist);
        let for_policy = Arc::clone(&allowlist);
        let policy = Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if Self::url_allowed(&for_policy, attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("redirect target is outside the download allowlist")
            }
        });
        let inner = reqwest::Client::builder()
            .redirect(policy)
            .timeout(OVERALL_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| NetError::Http(format!("client build: {e}")))?;
        Ok(HttpsClient { inner, allowlist })
    }

    fn url_allowed(allowlist: &HostAllowlist, url: &Url) -> bool {
        if url.scheme() != "https" {
            return false;
        }
        match url.host_str() {
            Some(host) => allowlist.is_allowed(host),
            None => false,
        }
    }

    fn check_url(&self, url: &str) -> Result<(), NetError> {
        ensure_https(url)?;
        let parsed = Url::parse(url).map_err(|e| NetError::Http(format!("url parse: {e}")))?;
        if !Self::url_allowed(&self.allowlist, &parsed) {
            return Err(NetError::DisallowedHost(url.to_string()));
        }
        Ok(())
    }

    async fn open(&self, url: &str) -> Result<reqwest::Response, NetError> {
        self.check_url(url)?;
        let resp = self
            .inner
            .get(url)
            .send()
            .await
            .map_err(|e| NetError::Http(format!("request {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(NetError::Http(format!(
                "request {url}: http {}",
                resp.status().as_u16()
            )));
        }
        if let Some(len) = resp.content_length()
            && len > MAX_TRANSFER_BYTES
        {
            return Err(NetError::TooLarge);
        }
        Ok(resp)
    }
}

impl HttpClient for HttpsClient {
    async fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
        let resp = self.open(url).await?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| NetError::Http(format!("body {url}: {e}")))?;
        if bytes.len() as u64 > MAX_TRANSFER_BYTES {
            return Err(NetError::TooLarge);
        }
        Ok(bytes.to_vec())
    }

    async fn download(&self, url: &str, dest: &Path) -> Result<u64, NetError> {
        let resp = self.open(url).await?;

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| NetError::Http(format!("mkdir {}: {e}", parent.display())))?;
        }

        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| NetError::Http(format!("create {}: {e}", dest.display())))?;

        let mut stream = resp.bytes_stream();
        let mut transferred: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| NetError::Http(format!("body {url}: {e}")))?;
            transferred += chunk.len() as u64;
            if transferred > MAX_TRANSFER_BYTES {
                return Err(NetError::TooLarge);
            }
            use tokio::io::AsyncWriteExt;
            file.write_all(&chunk)
                .await
                .map_err(|e| NetError::Http(format!("write {}: {e}", dest.display())))?;
        }
        use tokio::io::AsyncWriteExt;
        file.flush()
            .await
            .map_err(|e| NetError::Http(format!("flush {}: {e}", dest.display())))?;
        Ok(transferred)
    }
}
