use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::Method;
use serde_json::Value;

pub struct EvotownClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl EvotownClient {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        Self::with_timeout(base_url, api_key, std::time::Duration::from_secs(120))
    }

    pub fn with_timeout(
        base_url: &str,
        api_key: &str,
        timeout: std::time::Duration,
    ) -> Result<Self> {
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        })
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None)
    }

    pub fn post_json(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(body))
    }

    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.resolve_url(path);
        let response = self
            .http
            .get(&url)
            .header("Accept", "*/*")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .with_context(|| format!("GET {url} failed"))?;
        let status = response.status();
        let body = response.bytes().context("failed to read response body")?;
        if !status.is_success() {
            anyhow::bail!(
                "HTTP {} {}: {}",
                status.as_u16(),
                url,
                clip_error_body(&String::from_utf8_lossy(&body))
            );
        }
        Ok(body.to_vec())
    }

    fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let url = self.resolve_url(path);
        let mut builder = self
            .http
            .request(method.clone(), &url)
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key));
        if let Some(payload) = body {
            builder = builder.json(&payload);
        }
        let response = builder
            .send()
            .with_context(|| format!("{method} {url} failed"))?;
        let status = response.status();
        let text = response.text().context("failed to read response body")?;
        if !status.is_success() {
            anyhow::bail!(
                "HTTP {} {}: {}",
                status.as_u16(),
                url,
                clip_error_body(&text)
            );
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).with_context(|| format!("invalid JSON from {url}"))
    }

    fn resolve_url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            return path.to_string();
        }
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }
}

pub fn check_evotown_connectivity(client: &EvotownClient) -> Result<EvotownHealthReport> {
    let health = client.get_json("/health")?;
    let gateway = client
        .get_json("/api/gateway/v1/health")
        .unwrap_or(Value::Null);
    Ok(EvotownHealthReport { health, gateway })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvotownHealthReport {
    pub health: Value,
    pub gateway: Value,
}

/// A failed response body, cut to 500 bytes for the error message.
fn clip_error_body(text: &str) -> &str {
    crate::text::clip_bytes(text, 500)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cjk_error_body_does_not_panic() {
        // The clip used a raw byte index, so a Chinese body cut inside a
        // character and panicked while building the error message.
        let body = "错".repeat(300);
        let clipped = clip_error_body(&body);
        assert!(clipped.len() <= 500);
        assert!(body.starts_with(clipped));
    }

    #[test]
    fn a_lossy_error_body_does_not_panic() {
        // A body that is not valid UTF-8 becomes replacement characters, three
        // bytes each, so the byte index lands inside one of them.
        let body = vec![0xFF_u8; 600];
        let text = String::from_utf8_lossy(&body);
        let clipped = clip_error_body(&text);
        assert!(clipped.len() <= 500);
    }
}
