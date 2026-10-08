//! Fetches public tweets from X's syndication endpoint (the one behind embedded tweets).
//! No login, no guest token. Age-restricted and protected tweets come back as tombstones.

use serde::Deserialize;
use uet_core::Provider;

use crate::post::{FetchError, Media, Post, check_status};

/// Discord reportedly stops inlining linked videos somewhere between 50 and 200 MB; stay well
/// below by picking the best variant whose estimated size fits.
const MAX_EMBED_VIDEO_BYTES: u64 = 100_000_000;

pub struct Twitter {
    api: reqwest::Client,
}

impl Twitter {
    pub fn new(api: reqwest::Client) -> Self {
        Self { api }
    }

    pub async fn fetch_post(&self, id: &str) -> Result<Post, FetchError> {
        let resp = self
            .api
            .get("https://cdn.syndication.twimg.com/tweet-result")
            // `token` is required to be present but is not validated.
            .query(&[("id", id), ("lang", "en"), ("token", "a")])
            .send()
            .await?;
        let tweet: Tweet = check_status(resp)?.json().await?;
        if tweet.typename != "Tweet" {
            return Err(FetchError::NotFound);
        }
        tweet.into_post(id)
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Tweet {
    #[serde(rename = "__typename")]
    typename: String,
    id_str: String,
    text: String,
    lang: Option<String>,
    created_at: Option<String>,
    in_reply_to_screen_name: Option<String>,
    user: Option<User>,
    favorite_count: Option<u64>,
    conversation_count: Option<u64>,
    /// Quoted tweets carry `reply_count` instead of `conversation_count`.
    reply_count: Option<u64>,
    entities: Entities,
    #[serde(rename = "mediaDetails")]
    media_details: Vec<MediaDetail>,
    quoted_tweet: Option<Box<Tweet>>,
}

#[derive(Deserialize)]
struct User {
    name: String,
    screen_name: String,
    profile_image_url_https: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entities {
    urls: Vec<UrlEntity>,
    media: Vec<UrlEntity>,
}

#[derive(Deserialize)]
struct UrlEntity {
    url: String,
    expanded_url: Option<String>,
}

#[derive(Deserialize)]
struct MediaDetail {
    /// `photo`, `video` or `animated_gif`.
    #[serde(rename = "type", default)]
    kind: String,
    media_url_https: String,
    ext_alt_text: Option<String>,
    original_info: Option<Size>,
    video_info: Option<VideoInfo>,
}

#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct VideoInfo {
    duration_millis: Option<u64>,
    variants: Vec<Variant>,
}

#[derive(Deserialize)]
struct Variant {
    content_type: String,
    url: String,
    bitrate: Option<u64>,
}

impl Tweet {
    fn into_post(self, id: &str) -> Result<Post, FetchError> {
        let user = self.user.ok_or_else(|| FetchError::Parse("tweet has no user".into()))?;
        let quote = self
            .quoted_tweet
            .and_then(|q| {
                let id = q.id_str.clone();
                q.into_post(&id).ok()
            })
            .map(Box::new);
        let mut media: Vec<Media> = self.media_details.iter().map(MediaDetail::to_media).collect();
        // Text-only quote of a media tweet: show the quoted media.
        if media.is_empty() {
            media = quote.as_ref().map(|q| q.media.clone()).unwrap_or_default();
        }
        Ok(Post {
            provider: Provider::Twitter,
            id: id.to_owned(),
            handle: user.screen_name,
            display_name: Some(user.name).filter(|n| !n.is_empty()),
            text: Some(expand_text(&self.text, &self.entities)).filter(|t| !t.is_empty()),
            // `und`/`zxx` mark undetectable text (emoji only, bare links).
            lang: self.lang.filter(|l| !matches!(l.as_str(), "und" | "zxx" | "qme" | "qam")),
            created_at: self.created_at,
            // `_normal` is 48px; FxTwitter uses the 200px rendition.
            avatar: user.profile_image_url_https.map(|u| u.replace("_normal.", "_200x200.")),
            reply_to: self.in_reply_to_screen_name,
            likes: self.favorite_count,
            comments: self.conversation_count.or(self.reply_count),
            media,
            quote,
        })
    }
}

impl MediaDetail {
    fn to_media(&self) -> Media {
        let (width, height) = self.original_info.as_ref().map_or((0, 0), |s| (s.width, s.height));
        Media {
            video: self.video_info.as_ref().and_then(pick_variant).map(|v| v.url.clone()),
            image: self.media_url_https.clone(),
            width,
            height,
            alt: self.ext_alt_text.clone().filter(|a| !a.is_empty()),
            gif: self.kind == "animated_gif",
        }
    }
}

/// Highest-bitrate MP4 that fits the embed budget, else the smallest MP4 (HLS is skipped).
fn pick_variant(info: &VideoInfo) -> Option<&Variant> {
    let mp4 = || info.variants.iter().filter(|v| v.content_type == "video/mp4");
    let fits = |v: &&Variant| match (v.bitrate, info.duration_millis) {
        (Some(bps), Some(ms)) => bps.saturating_mul(ms) / 8_000 <= MAX_EMBED_VIDEO_BYTES,
        _ => true,
    };
    mp4()
        .filter(fits)
        .max_by_key(|v| v.bitrate.unwrap_or(0))
        .or_else(|| mp4().min_by_key(|v| v.bitrate.unwrap_or(u64::MAX)))
}

/// Expands t.co links, drops the trailing t.co link that points at the attached media, and
/// decodes the `&amp;`/`&lt;`/`&gt;` escapes X applies to tweet text.
fn expand_text(text: &str, entities: &Entities) -> String {
    let mut out = text.to_owned();
    for e in &entities.media {
        out = out.replace(&e.url, "");
    }
    for e in &entities.urls {
        if let Some(expanded) = &e.expanded_url {
            out = out.replace(&e.url, expanded);
        }
    }
    out.trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(bitrate: Option<u64>, content_type: &str) -> Variant {
        Variant {
            content_type: content_type.into(),
            url: format!("{bitrate:?}"),
            bitrate,
        }
    }

    #[test]
    fn picks_best_mp4_within_budget() {
        let variants = vec![
            variant(None, "application/x-mpegURL"),
            variant(Some(832_000), "video/mp4"),
            variant(Some(10_368_000), "video/mp4"),
            variant(Some(2_176_000), "video/mp4"),
        ];
        // 9 s: everything fits, take the best.
        let short = VideoInfo { duration_millis: Some(9_000), variants };
        assert_eq!(pick_variant(&short).unwrap().bitrate, Some(10_368_000));
        // 5 min: 1080p (~389 MB) is over budget, 720p (~82 MB) fits.
        let long = VideoInfo { duration_millis: Some(300_000), ..short };
        assert_eq!(pick_variant(&long).unwrap().bitrate, Some(2_176_000));
        // 2 h: nothing fits, fall back to the smallest MP4 rather than HLS.
        let huge = VideoInfo { duration_millis: Some(7_200_000), ..long };
        assert_eq!(pick_variant(&huge).unwrap().bitrate, Some(832_000));
    }

    #[test]
    fn expands_links_and_strips_media_link() {
        let entities = Entities {
            urls: vec![UrlEntity {
                url: "https://t.co/E9JTPkhWqF".into(),
                expanded_url: Some("https://nextjs.org/13-2".into()),
            }],
            media: vec![UrlEntity {
                url: "https://t.co/D68z4K2wq7".into(),
                expanded_url: None,
            }],
        };
        assert_eq!(
            expand_text("Read https://t.co/E9JTPkhWqF https://t.co/D68z4K2wq7", &entities),
            "Read https://nextjs.org/13-2"
        );
        // `&amp;lt;` is a literal "&lt;" typed by the author.
        assert_eq!(
            expand_text("a &amp; b &lt;3 &amp;lt;", &Entities::default()),
            "a & b <3 &lt;"
        );
    }
}
