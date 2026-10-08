//! Provider-neutral post model and fetch errors.

use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::StatusCode;
use uet_core::Provider;

#[derive(Debug, Clone)]
pub struct Post {
    pub provider: Provider,
    /// Instagram shortcode or tweet id.
    pub id: String,
    /// Username / screen name, without `@`.
    pub handle: String,
    pub display_name: Option<String>,
    pub text: Option<String>,
    /// ISO 639-1 language as detected by X; `None` on Instagram.
    pub lang: Option<String>,
    /// ISO 8601 UTC timestamp.
    pub created_at: Option<String>,
    pub avatar: Option<String>,
    /// Screen name this post replies to.
    pub reply_to: Option<String>,
    pub likes: Option<u64>,
    /// Comments (Instagram) or replies (X).
    pub comments: Option<u64>,
    /// Empty for text-only tweets. A text-only quote tweet carries the quoted media here.
    pub media: Vec<Media>,
    pub quote: Option<Box<Post>>,
}

#[derive(Debug, Clone)]
pub struct Media {
    pub video: Option<String>,
    /// Photo, or the cover frame for videos.
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub alt: Option<String>,
    /// X animated GIF (served as a looping mp4).
    pub gif: bool,
}

impl Post {
    /// Canonical post URL on the original site.
    pub fn url(&self) -> String {
        match self.provider {
            Provider::Instagram => format!("https://www.instagram.com/p/{}/", self.id),
            Provider::Twitter => format!("https://x.com/{}/status/{}", self.handle, self.id),
        }
    }

    pub fn author_url(&self) -> String {
        match self.provider {
            Provider::Instagram => format!("https://www.instagram.com/{}/", self.handle),
            Provider::Twitter => format!("https://x.com/{}", self.handle),
        }
    }

    /// `Name (@handle)`, or `@handle` without a display name.
    pub fn title(&self) -> String {
        match &self.display_name {
            Some(name) => format!("{name} (@{})", self.handle),
            None => format!("@{}", self.handle),
        }
    }

    /// Photos that make up the mosaic (FxTwitter's `media.mosaic`): the first four non-video
    /// items, when there are at least two.
    pub fn mosaic_photos(&self) -> Option<Vec<&Media>> {
        let photos: Vec<&Media> = self.media.iter().filter(|m| m.video.is_none()).take(4).collect();
        (photos.len() >= 2).then_some(photos)
    }

    /// Tile URLs and their expected pixel sizes. X tiles use the `large` rendition (fits in
    /// 2048×2048), like FxEmbed's mosaic service, which bounds decode memory.
    pub fn mosaic_tiles(&self) -> Option<Vec<(String, (u32, u32))>> {
        const LARGE: u32 = 2048;
        let tiles = self.mosaic_photos()?.into_iter().map(|m| match self.provider {
            Provider::Twitter => {
                let longest = m.width.max(m.height);
                let size = if longest > LARGE {
                    let scale = |d: u32| (u64::from(d) * u64::from(LARGE)).div_ceil(u64::from(longest)) as u32;
                    (scale(m.width), scale(m.height))
                } else {
                    (m.width, m.height)
                };
                (sized_image(self.provider, &m.image, "large"), size)
            }
            Provider::Instagram => (m.image.clone(), (m.width, m.height)),
        });
        Some(tiles.collect())
    }

    /// Pixel size of the rendered mosaic.
    pub fn mosaic_size(&self) -> Option<(u32, u32)> {
        let sizes: Vec<(u32, u32)> = self.mosaic_tiles()?.into_iter().map(|(_, s)| s).collect();
        crate::mosaic::canvas_size(&sizes)
    }

    /// Time until the earliest signed CDN URL (Instagram's `oe=` hex unix timestamp) expires.
    /// `None` when no URL carries an expiry (X media URLs don't).
    pub fn url_lifetime(&self) -> Option<Duration> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        self.media
            .iter()
            .flat_map(|m| m.video.iter().chain([&m.image]))
            .chain(self.avatar.iter())
            .filter_map(|u| cdn_expiry(u))
            .min()
            // Refresh well before the CDN starts returning 403s.
            .map(|oe| Duration::from_secs(oe.saturating_sub(now).saturating_sub(600)))
    }
}

/// Photo URL at a named size. X serves `small|medium|large|orig|4096x4096` via `?name=`;
/// Instagram URLs are signed and returned unchanged.
pub fn sized_image(provider: Provider, url: &str, name: &str) -> String {
    match provider {
        Provider::Twitter if !url.contains('?') => format!("{url}?name={name}"),
        _ => url.to_owned(),
    }
}

/// Formats a unix timestamp as ISO 8601 UTC (`2024-01-31T12:00:00.000Z`).
pub fn iso8601(unix: u64) -> String {
    let (days, secs) = (unix / 86_400, unix % 86_400);
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn cdn_expiry(url: &str) -> Option<u64> {
    let query = url.split_once('?')?.1;
    let hex = query.split('&').find_map(|kv| kv.strip_prefix("oe="))?;
    u64::from_str_radix(hex, 16).ok()
}

#[derive(Debug)]
pub enum FetchError {
    NotFound,
    /// Rate limited or login-walled; the server IP is likely flagged.
    Blocked(StatusCode),
    Http(reqwest::Error),
    Parse(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("post not found, deleted, private or age-restricted"),
            Self::Blocked(s) => write!(f, "blocked upstream (HTTP {s})"),
            Self::Http(e) => write!(f, "request failed: {e}"),
            Self::Parse(e) => write!(f, "unexpected response: {e}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<reqwest::Error> for FetchError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for FetchError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e.to_string())
    }
}

pub fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, FetchError> {
    match resp.status() {
        s if s.is_success() => Ok(resp),
        StatusCode::NOT_FOUND => Err(FetchError::NotFound),
        s => Err(FetchError::Blocked(s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_iso8601() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        // Leap day, and the March boundary of the civil-calendar shift.
        assert_eq!(iso8601(951_825_600), "2000-02-29T12:00:00.000Z");
        assert_eq!(iso8601(1_709_251_199), "2024-02-29T23:59:59.000Z");
        assert_eq!(iso8601(1_709_251_200), "2024-03-01T00:00:00.000Z");
    }
}
