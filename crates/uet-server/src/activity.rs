//! FxTwitter's Discord "activity" embeds: the page links a fake Mastodon status and Discord
//! renders `/api/v1/statuses/{snowcode}` as a Mastodon post (text with links, all media,
//! engagement). Port of FxEmbed's `src/embed/activity.ts` and `src/helpers/snowcode.ts` (MIT).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uet_core::Provider;

use crate::{
    embed::{self, ACTIVITY_BR, escape, format_number, linkify},
    post::{Media, Post},
    translate::Translated,
};

/// Snowcode alphabet; each character becomes its two-digit index.
const ALPHABET: &[u8] = br#"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789{}[]":,.-_"#;

/// Mastodon attachment id FxEmbed uses for every attachment.
const ATTACHMENT_ID: &str = "114163769487684704";

/// Status options carried through Discord's request in the all-digit status id.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snowcode {
    /// Post id.
    pub i: String,
    /// Translation target (DeepL code).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l: Option<String>,
    /// Text only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<u8>,
    /// Force mosaic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m: Option<u8>,
    /// 1-based media item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Realm from [`realm`]; absent for X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<String>,
}

impl Snowcode {
    /// `None` if a value has characters outside the alphabet.
    pub fn encode(&self) -> Option<String> {
        let json = serde_json::to_string(self).ok()?;
        let inner = json.strip_prefix('{')?.strip_suffix('}')?;
        inner
            .bytes()
            .map(|b| ALPHABET.iter().position(|&a| a == b).map(|i| format!("{i:02}")))
            .collect()
    }

    pub fn decode(code: &str) -> Option<Self> {
        if code.is_empty() || code.len() % 2 != 0 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut json = String::with_capacity(code.len() / 2 + 2);
        json.push('{');
        for pair in code.as_bytes().chunks_exact(2) {
            let i = usize::from((pair[0] - b'0') * 10 + (pair[1] - b'0'));
            json.push(char::from(*ALPHABET.get(i)?));
        }
        json.push('}');
        serde_json::from_str(&json).ok()
    }

    pub fn provider(&self) -> Option<Provider> {
        match self.r.as_deref() {
            None => Some(Provider::Twitter),
            Some(r) => provider_for_realm(r),
        }
    }
}

/// Short realm tag for non-X posts in snowcodes and `owoembed` links.
pub fn realm(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Twitter => None,
        Provider::Instagram => Some("ig"),
    }
}

pub fn provider_for_realm(realm: &str) -> Option<Provider> {
    match realm {
        "ig" => Some(Provider::Instagram),
        _ => None,
    }
}

/// Mastodon v1 status for `post` with the snowcode's options applied.
pub fn status(post: &Post, code: &Snowcode, translation: Option<&Translated>, base: &str) -> Value {
    let url = post.url();
    let avatar = post.avatar.as_deref();
    json!({
        "id": post.id,
        "url": url,
        "uri": url,
        "created_at": post.created_at,
        "edited_at": null,
        "reblog": null,
        "in_reply_to_id": null,
        "in_reply_to_account_id": null,
        "language": post.lang,
        "content": content(post, translation),
        "spoiler_text": "",
        "visibility": "public",
        "application": { "name": embed::brand(post.provider), "website": null },
        "media_attachments": attachments(post, code, base),
        "account": {
            "id": post.handle,
            "display_name": post.display_name.as_deref().unwrap_or(&post.handle),
            "username": post.handle,
            "acct": post.handle,
            "url": post.author_url(),
            "uri": post.author_url(),
            "created_at": post.created_at,
            "locked": false,
            "bot": false,
            "discoverable": true,
            "indexable": false,
            "group": false,
            "avatar": avatar,
            "avatar_static": avatar,
            "header": null,
            "header_static": null,
            "followers_count": 0,
            "following_count": 0,
            "statuses_count": 0,
            "hide_collections": false,
            "noindex": false,
            "emojis": [],
            "roles": [],
            "fields": [],
        },
        "mentions": [],
        "tags": [],
        "emojis": [],
        "card": null,
        "poll": null,
    })
}

/// Escaped, linkified text with Discord-safe line breaks.
fn format_text(text: &str, provider: Provider) -> String {
    linkify(text.trim(), provider).replace('\n', ACTIVITY_BR)
}

/// FxEmbed's `getStatusText`.
fn content(post: &Post, translation: Option<&Translated>) -> String {
    let original = format_text(post.text.as_deref().unwrap_or_default(), post.provider);
    let mut out = match translation {
        Some(Translated { source, text: Some(translated), .. }) => format!(
            "<b>📑 Translated from {}</b><br><br>{}<br><br><blockquote><b>Original text</b><br>{original}</blockquote>",
            escape(source),
            format_text(translated, post.provider),
        ),
        _ => format!("{original}<br><br>"),
    };
    if let Some(quote) = &post.quote {
        let text = translation
            .and_then(|t| t.quote.as_deref())
            .or(quote.text.as_deref())
            .unwrap_or_default();
        out.push_str(&format!(
            "<blockquote><b><a href=\"{}\">Quoting</a> {} (<a href=\"{}\">@{}</a>)</b><br>\u{FE00}<br>{}</blockquote>",
            escape(&quote.url()),
            escape(quote.display_name.as_deref().unwrap_or(&quote.handle)),
            escape(&quote.author_url()),
            escape(&quote.handle),
            format_text(text, quote.provider),
        ));
    }
    if let Some(to) = &post.reply_to {
        out = format!(
            "<sub>↩ <a href=\"https://x.com/{0}\" class=\"u-url mention\">(@{0})</a></sub><br>{out}",
            escape(to)
        );
    }
    let social: String = embed::engagement(post)
        .into_iter()
        .map(|(emoji, n, intent)| match intent {
            Some(href) => format!("<a href=\"{}\">{emoji}</a> {}&ensp;", escape(&href), format_number(n)),
            None => format!("{emoji} {}&ensp;", format_number(n)),
        })
        .collect();
    if !social.is_empty() {
        out.push_str(&format!("<b>{social}</b>"));
    }
    out
}

fn attachments(post: &Post, code: &Snowcode, base: &str) -> Vec<Value> {
    if code.t.is_some() {
        return Vec::new();
    }
    let all: Vec<(usize, &Media)> = (1..).zip(&post.media).collect();
    let picked = match code.n.and_then(|n| all.get((n as usize).checked_sub(1)?)) {
        Some(&item) => vec![item],
        None => all,
    };
    if code.m.is_some()
        && picked.len() != 1
        && let Some((w, h)) = post.mosaic_size()
    {
        return vec![json!({
            "id": ATTACHMENT_ID,
            "type": "image",
            "url": embed::mosaic_url(base, post),
            "preview_url": null,
            "remote_url": null,
            "preview_remote_url": null,
            "text_url": null,
            "description": null,
            "meta": { "original": { "width": w, "height": h, "size": format!("{w}x{h}"), "aspect": f64::from(w) / f64::from(h) } },
        })];
    }
    picked
        .into_iter()
        .map(|(n, m)| {
            let (kind, url, preview, scale) = match m.video {
                None => ("image", embed::media_url(base, post, n, "image.jpg"), None, 1.0),
                Some(_) => {
                    // Discord rejects huge videos and shows tiny ones tiny.
                    let mut scale = 1.0;
                    if m.width > 1920 || m.height > 1920 {
                        scale = 0.5;
                    }
                    if m.width < 400 || m.height < 400 {
                        scale = 2.0;
                    }
                    (
                        "video",
                        embed::media_url(base, post, n, "video.mp4"),
                        Some(embed::media_url(base, post, n, "image.jpg")),
                        scale,
                    )
                }
            };
            let scaled = |d: u32| (f64::from(d) * scale).round() as u32;
            let (w, h) = (scaled(m.width), scaled(m.height));
            json!({
                "id": ATTACHMENT_ID,
                "type": kind,
                "url": url,
                "preview_url": preview,
                "remote_url": null,
                "preview_remote_url": null,
                "text_url": null,
                "description": m.alt,
                "meta": { "original": {
                    "width": w,
                    "height": h,
                    "size": format!("{w}x{h}"),
                    "aspect": f64::from(m.width) / f64::from(m.height),
                } },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowcode_round_trips() {
        let code = Snowcode {
            i: "DeMPFloOrjv-_".into(),
            l: Some("pt-br".into()),
            t: Some(1),
            n: Some(3),
            r: Some("ig".into()),
            ..Snowcode::default()
        };
        let encoded = code.encode().unwrap();
        assert!(encoded.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(Snowcode::decode(&encoded), Some(code));
        // FxEmbed's encoding of {"i":"20"}.
        assert_eq!(
            Snowcode { i: "20".into(), ..Snowcode::default() }.encode().as_deref(),
            Some("6608666766545266")
        );
        assert_eq!(Snowcode::decode("6608666766545266").unwrap().provider(), Some(Provider::Twitter));
        assert!(Snowcode { i: "a b".into(), ..Snowcode::default() }.encode().is_none());
        assert!(Snowcode::decode("999").is_none());
        assert!(Snowcode::decode("9999").is_none());
    }
}
