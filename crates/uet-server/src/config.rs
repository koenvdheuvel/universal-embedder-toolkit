use std::{env, net::SocketAddr, time::Duration};

/// How `/media/...` URLs hand the file to the embedding client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaMode {
    /// 302 to the CDN URL (signed, for Instagram). No bandwidth cost.
    Redirect,
    /// Stream the file through this server (Range requests forwarded).
    Proxy,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Public base URL without trailing slash, e.g. `https://fix.example.com`.
    pub public_url: String,
    /// Host of `public_url`; modifier subdomains (`d.`, `t.`, ...) are matched against it.
    pub public_host: String,
    /// GraphQL `doc_id` of `PolarisPostActionLoadPostQueryQuery`; Instagram rotates these.
    pub doc_id: String,
    /// Optional HTTP/SOCKS5 proxy for Instagram/X API requests (not media).
    pub upstream_proxy: Option<String>,
    pub media_mode: MediaMode,
    /// Enables `/{lang}` translation.
    pub deepl_api_key: Option<String>,
    pub cache_ttl: Duration,
    pub cache_capacity: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| env::var(k).ok().filter(|v| !v.trim().is_empty());
        let parse_u64 = |k: &str, default: u64| -> Result<u64, String> {
            var(k).map_or(Ok(default), |v| v.parse().map_err(|e| format!("{k}: {e}")))
        };
        let media_mode = match var("MEDIA_MODE").as_deref() {
            None | Some("redirect") => MediaMode::Redirect,
            Some("proxy") => MediaMode::Proxy,
            Some(other) => return Err(format!("MEDIA_MODE: expected redirect|proxy, got {other}")),
        };
        let public_url = var("PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();
        let public_host = url::Url::parse(&public_url)
            .map_err(|e| format!("PUBLIC_URL: {e}"))?
            .host_str()
            .ok_or("PUBLIC_URL: missing host")?
            .to_ascii_lowercase();
        Ok(Self {
            bind: var("BIND")
                .unwrap_or_else(|| "0.0.0.0:8080".into())
                .parse()
                .map_err(|e| format!("BIND: {e}"))?,
            public_url,
            public_host,
            doc_id: var("IG_DOC_ID").unwrap_or_else(|| "25018359077785073".into()),
            upstream_proxy: var("UPSTREAM_PROXY"),
            media_mode,
            deepl_api_key: var("DEEPL_API_KEY"),
            cache_ttl: Duration::from_secs(parse_u64("CACHE_TTL_SECS", 3600)?),
            cache_capacity: parse_u64("CACHE_CAPACITY", 10_000)?,
        })
    }
}
