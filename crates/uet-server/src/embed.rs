//! FxTwitter-style embed pages: OpenGraph/Twitter-card HTML, Telegram Instant View, and the
//! oEmbed JSON Discord reads for its author/provider lines.
//!
//! The rules follow FxEmbed's `src/embed/status.ts` and `src/render/*.ts`
//! (MIT, <https://github.com/FxEmbed/FxEmbed>).

use std::fmt::Write;

use serde::Serialize;
use uet_core::Provider;

use crate::{
    activity::{self, Snowcode},
    post::{Media, Post},
    translate::{Translated, same_language},
};

/// FxEmbed's `Strings.DEFAULT_AUTHOR_TEXT`.
pub const DEFAULT_AUTHOR_TEXT: &str = "Embed";

/// FxTwitter's `BOT_UA_REGEX` terms, matched case-insensitively against the User-Agent.
/// `firefox/38` is Discord's secondary unfurler.
const BOT_TERMS: &[&str] = &[
    "bot",
    "facebook",
    "embed",
    "got",
    "firefox/92",
    "firefox/38",
    "chrome/96.0.4664.110",
    "curl",
    "wget",
    "go-http",
    "yahoo",
    "generator",
    "whatsapp",
    "revoltchat",
    "preview",
    "link",
    "proxy",
    "vkshare",
    "images",
    "analyzer",
    "index",
    "crawl",
    "spider",
    "python",
    "node",
    "deno",
    "mastodon",
    "http.rb",
    "ruby",
    "bun/",
    "fiddler",
    "iframely",
    "steamchaturllookup",
    "bluesky",
    "matrix-media-repo",
    "cardyb",
    "resolver",
    "util",
    "feedly",
    "rss",
    "reader",
    "atom",
    "thunderbird",
    "axios",
];

/// Modifier subdomains, FxTwitter's `*_DOMAINS` lists.
const HOST_LABELS: &[&str] = &["d", "dl", "t", "i", "g", "m", "o"];

/// Text separator FxEmbed puts after Activity line breaks (two U+FE00 variation selectors)
/// so Discord keeps blank lines.
pub const ACTIVITY_BR: &str = "<br>\u{FE00}\u{FE00}";

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

pub fn is_bot(user_agent: &str) -> bool {
    BOT_TERMS.iter().any(|term| contains_ci(user_agent, term))
}

/// Which unfurler is asking; FxTwitter special-cases these three.
#[derive(Clone, Copy, Debug, Default)]
pub struct Agent {
    pub telegram: bool,
    pub discord: bool,
    /// Shows several `og:image` tags as a gallery (`NATIVE_MULTI_IMAGE_UA_REGEX`).
    pub native_multi_image: bool,
}

impl Agent {
    pub fn new(user_agent: &str) -> Self {
        Self {
            telegram: user_agent.contains("TelegramBot"),
            discord: user_agent.contains("Discordbot"),
            native_multi_image: contains_ci(user_agent, "discordbot/")
                || contains_ci(user_agent, "matrixpreviewbot"),
        }
    }
}

/// FxTwitter's link modifiers. At most one is set, as in FxTwitter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `d.`/`dl.`, `.mp4`/`.jpg` suffix, `/dl/`: redirect to the media file.
    pub direct: bool,
    /// `t.`: no media.
    pub text_only: bool,
    /// `g.`: media and author only.
    pub gallery: bool,
    /// `m.`: one mosaic image even where several images are supported.
    pub force_mosaic: bool,
    /// `o.`: no Discord activity embed.
    pub no_activity: bool,
    /// `i.`: Telegram Instant View for every post.
    pub instant_view: bool,
}

impl Flags {
    /// `label` comes from [`host_label`]. A direct-media path wins over the subdomain.
    pub fn new(label: &str, direct_path: bool) -> Self {
        let mut flags = Self::default();
        match label {
            _ if direct_path => flags.direct = true,
            "d" | "dl" => flags.direct = true,
            "t" => flags.text_only = true,
            "i" => flags.instant_view = true,
            "g" => flags.gallery = true,
            "m" => flags.force_mosaic = true,
            "o" => flags.no_activity = true,
            _ => {}
        }
        flags
    }
}

/// Modifier label of a `Host` header relative to the public host: `Some("")` for the public
/// host itself, `Some("d")` for `d.{public_host}`, `None` for any other host.
pub fn host_label(host: &str, public_host: &str) -> Option<&'static str> {
    // Strip `:port`, but not the colons of a bracketed IPv6 literal.
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    if host.eq_ignore_ascii_case(public_host) {
        return Some("");
    }
    let (label, rest) = host.split_once('.')?;
    if !rest.eq_ignore_ascii_case(public_host) {
        return None;
    }
    HOST_LABELS
        .iter()
        .find(|l| l.eq_ignore_ascii_case(label))
        .copied()
}

/// Origin of the original site.
pub fn site_origin(provider: Provider) -> &'static str {
    match provider {
        Provider::Instagram => "https://www.instagram.com",
        Provider::Twitter => "https://x.com",
    }
}

/// Brand shown as the site name; FxTwitter shows its own.
pub fn brand(provider: Provider) -> &'static str {
    match provider {
        Provider::Instagram => "Instagram",
        Provider::Twitter => "X",
    }
}

fn theme_color(provider: Provider) -> &'static str {
    match provider {
        Provider::Instagram => "#E1306C",
        Provider::Twitter => "#1D9BF0",
    }
}

/// `{base}/media/{provider}/{id}/{n}/{file}`; `n` is 1-based.
pub fn media_url(base: &str, post: &Post, n: usize, file: &str) -> String {
    format!("{base}/media/{}/{}/{n}/{file}", post.provider.slug(), post.id)
}

pub fn mosaic_url(base: &str, post: &Post) -> String {
    format!("{base}/mosaic/{}/{}.jpg", post.provider.slug(), post.id)
}

/// One embed request.
pub struct Page<'a> {
    pub post: &'a Post,
    pub flags: Flags,
    pub agent: Agent,
    /// 0-based media item from `/photo/N`, in range.
    pub index: Option<usize>,
    pub translation: Option<&'a Translated>,
    /// DeepL target from the `/{lang}` suffix.
    pub lang: Option<&'static str>,
    /// `PUBLIC_URL`, for media, mosaic and oEmbed links.
    pub base: &'a str,
    /// Origin the request came in on, so the activity link keeps its modifier subdomain.
    pub origin: &'a str,
}

impl Page<'_> {
    fn gallery(&self) -> bool {
        // Gallery without media falls back to the normal embed.
        self.flags.gallery && !self.post.media.is_empty()
    }

    /// Discord gets FxTwitter's Mastodon "activity" embed (`/api/v1/statuses/...`) and the
    /// page skips its own media tags.
    pub fn uses_activity(&self) -> bool {
        self.agent.discord && !self.flags.direct && !self.gallery() && !self.flags.no_activity
    }

    fn uses_instant_view(&self) -> bool {
        let media = &self.post.media;
        self.agent.telegram
            && !self.flags.direct
            && !self.gallery()
            && (self.flags.instant_view
                || self.translation.is_some()
                || self.post.quote.is_some()
                // Telegram renders photos badly outside IV and previews only one video.
                || media.iter().any(|m| m.video.is_none())
                || media.iter().filter(|m| m.video.is_some()).count() > 1)
    }

    fn snowcode(&self) -> Option<String> {
        let post = self.post;
        Snowcode {
            i: post.id.clone(),
            l: self
                .lang
                .filter(|t| !post.lang.as_deref().is_some_and(|l| same_language(l, t)))
                .map(str::to_owned),
            t: self.flags.text_only.then_some(1),
            m: self.flags.force_mosaic.then_some(1),
            n: self.index.map(|i| i as u32 + 1),
            r: activity::realm(post.provider).map(str::to_owned),
        }
        .encode()
    }
}

#[derive(Default)]
struct Head(String);

impl Head {
    fn meta(&mut self, property: &str, content: &str) {
        self.meta_html(property, &escape(content));
    }

    /// `content` is already escaped HTML.
    fn meta_html(&mut self, property: &str, content: &str) {
        let _ = writeln!(self.0, r#"<meta property="{property}" content="{content}"/>"#);
    }

    fn link(&mut self, rel: &str, href: &str) {
        let _ = writeln!(self.0, r#"<link rel="{rel}" href="{}"/>"#, escape(href));
    }
}

/// Mutable bits the media renderers may override.
struct Lines {
    author_text: String,
    site_name: String,
    card: &'static str,
}

/// Renders the embed page for an unfurler.
pub fn render_html(page: &Page) -> String {
    let post = page.post;
    let flags = page.flags;
    let agent = page.agent;
    let gallery = page.gallery();
    let use_activity = page.uses_activity();
    let use_iv = page.uses_instant_view();
    let brand = brand(post.provider);
    let url = post.url();
    let title = post.title();
    let social = social_proof(post);

    let mut lines = Lines {
        author_text: social.clone().unwrap_or_else(|| DEFAULT_AUTHOR_TEXT.into()),
        site_name: brand.to_owned(),
        card: if post.media.iter().any(|m| m.video.is_some()) {
            "player"
        } else if post.media.is_empty() {
            "summary"
        } else {
            "summary_large_image"
        },
    };
    let mut h = Head::default();
    h.link("canonical", &url);
    h.meta("og:url", &url);
    if post.provider == Provider::Twitter {
        h.meta("twitter:site", &format!("@{}", post.handle));
        h.meta("twitter:creator", &format!("@{}", post.handle));
    }
    if !gallery {
        h.meta("theme-color", theme_color(post.provider));
        h.meta("twitter:title", &title);
    }
    // Telegram gets stuck on refresh tags.
    if !agent.telegram {
        let _ = writeln!(h.0, r#"<meta http-equiv="refresh" content="0;url={}"/>"#, escape(&url));
    }
    let body = if use_iv {
        h.meta("al:android:app_name", "Medium");
        if let Some(at) = &post.created_at {
            h.meta("article:published_time", at);
        }
        instant_view(page)
    } else {
        format!(r#"<a href="{0}">{0}</a>"#, escape(&url))
    };

    let mut text = match page.translation {
        Some(Translated { source, text: Some(translated), .. }) => {
            format!("📑 Translated from {source}\n\n{translated}\n\n")
        }
        _ => post.text.clone().unwrap_or_default(),
    };

    if !use_activity && !flags.text_only {
        let media = &post.media;
        if let Some(i) = page.index {
            render_media(&mut h, page, &mut lines, &text, i, true);
        } else if let Some(i) = media.iter().position(|m| m.video.is_some()) {
            render_media(&mut h, page, &mut lines, &text, i, false);
        } else if post.mosaic_photos().is_some() {
            if agent.native_multi_image && !flags.force_mosaic {
                for (i, m) in media.iter().enumerate().filter(|(_, m)| m.video.is_none()) {
                    photo_tags(&mut h, page, i + 1, m);
                }
            } else {
                let mosaic = mosaic_url(page.base, post);
                h.meta("twitter:image", &mosaic);
                h.meta("og:image", &mosaic);
                if let Some((w, hgt)) = post.mosaic_size() {
                    h.meta("og:image:width", &w.to_string());
                    h.meta("og:image:height", &hgt.to_string());
                }
            }
        } else if !media.is_empty() {
            render_media(&mut h, page, &mut lines, &text, 0, false);
        }
    }

    if let Some(quote) = &post.quote {
        text.push('\n');
        text.push_str(&quote_text(quote, page.translation.and_then(|t| t.quote.as_deref())));
    }
    if !agent.discord
        && let Some(to) = &post.reply_to
    {
        text = format!("↩ Replying to @{to}\n{text}");
    }

    if let Some(avatar) = &post.avatar {
        if post.media.is_empty() && !flags.text_only {
            if use_iv {
                h.meta("twitter:image", avatar);
            } else {
                h.meta("og:image", avatar);
                h.meta("twitter:image", "0");
            }
        }
        h.link("apple-touch-icon", avatar);
    }

    h.meta("twitter:card", lines.card);
    if !gallery {
        h.meta("og:title", &title);
        if use_iv {
            // Telegram ignores newlines in IV descriptions but honours `<br>`.
            h.meta_html("og:description", &escape(&text).replace('\n', "<br>"));
        } else {
            h.meta("og:description", &text);
        }
        h.meta("og:site_name", if use_activity { brand } else { &lines.site_name });
    } else if agent.telegram {
        h.meta("og:site_name", &title);
    } else {
        h.meta("og:title", &title);
    }

    if lines.author_text == DEFAULT_AUTHOR_TEXT
        && let Some(to) = &post.reply_to
    {
        lines.author_text = format!("↪ Replying to @{to}");
    }

    if !gallery {
        let _ = writeln!(
            h.0,
            r#"<link rel="alternate" href="{}" type="application/json+oembed" title="{}"/>"#,
            escape(&oembed_href(page, &lines, social.as_deref())),
            escape(post.display_name.as_deref().unwrap_or(&post.handle)),
        );
    }
    if use_activity && let Some(code) = page.snowcode() {
        let _ = writeln!(
            h.0,
            r#"<link href="{}/users/{}/statuses/{code}" rel="alternate" type="application/activity+json"/>"#,
            page.origin,
            escape(&encode(&post.handle)),
        );
    }

    let lang = post.lang.as_deref().unwrap_or("en");
    format!(
        "<!DOCTYPE html>\n<html lang=\"{}\"><head>\n<meta charset=\"utf-8\"/>\n{}</head><body>{body}</body></html>\n",
        escape(lang),
        h.0
    )
}

/// One media item's tags, with FxTwitter's photo counters and video author line.
fn render_media(h: &mut Head, page: &Page, lines: &mut Lines, text: &str, i: usize, is_override: bool) {
    let post = page.post;
    let media = &post.media;
    let m = &media[i];
    let (n, all) = (i + 1, media.len());
    let brand = brand(post.provider);
    if m.video.is_none() {
        let photos = media.iter().filter(|m| m.video.is_none()).count();
        // Without an override, multi-photo posts use the mosaic and get no counter.
        if photos > 1 && is_override {
            let kind = if photos == all { "Photo" } else { "Media" };
            let counter = format!("{kind} {n} / {all}");
            lines.author_text = if lines.author_text == DEFAULT_AUTHOR_TEXT || page.agent.telegram {
                counter.clone()
            } else {
                format!("{}   ―   {counter}", lines.author_text)
            };
            lines.site_name = match social_proof(post) {
                Some(engagement) if !page.agent.telegram => {
                    format!("{brand} - {engagement} - {counter}")
                }
                _ => format!("{brand} - {counter}"),
            };
        }
        lines.card = "summary_large_image";
        photo_tags(h, page, n, m);
        return;
    }

    let videos = media.iter().filter(|m| m.video.is_some()).count();
    if all > 1 && page.agent.telegram {
        let kind = if videos == all { "Video" } else { "Media" };
        lines.site_name = format!("{brand} - {kind} {n} / {all}");
    }
    // Video embeds hide the description, so the post text moves to the oEmbed author line.
    let mut author = page
        .translation
        .and_then(|t| t.text.clone())
        .unwrap_or_else(|| text.to_owned());
    if author.chars().count() < 40
        && let Some(quote) = &post.quote
    {
        author.push('\n');
        author.push_str(&quote_text(quote, page.translation.and_then(|t| t.quote.as_deref())));
    }
    if !author.is_empty() {
        lines.author_text = author;
    }
    lines.card = "player";

    // Discord refuses huge videos and shows tiny ones tiny.
    let scale = if m.width > 1920 || m.height > 1920 {
        0.5
    } else if m.width < 400 && m.height < 400 {
        2.0
    } else {
        1.0
    };
    let (w, hgt) = (
        (f64::from(m.width) * scale).to_string(),
        (f64::from(m.height) * scale).to_string(),
    );
    let video = media_url(page.base, post, n, "video.mp4");
    h.meta("twitter:player:height", &hgt);
    h.meta("twitter:player:width", &w);
    h.meta("twitter:player:stream", &video);
    h.meta("twitter:player:stream:content_type", "video/mp4");
    h.meta("og:video", &video);
    h.meta("og:video:secure_url", &video);
    h.meta("og:video:height", &hgt);
    h.meta("og:video:width", &w);
    h.meta("og:video:type", "video/mp4");
    h.meta("og:image", &media_url(page.base, post, n, "image.jpg"));
    h.meta("twitter:image", "0");
}

fn photo_tags(h: &mut Head, page: &Page, n: usize, m: &Media) {
    let image = media_url(page.base, page.post, n, "image.jpg");
    let (w, hgt) = (m.width.to_string(), m.height.to_string());
    h.meta("twitter:image", &image);
    h.meta("og:image", &image);
    h.meta("twitter:image:width", &w);
    h.meta("twitter:image:height", &hgt);
    h.meta("og:image:width", &w);
    h.meta("og:image:height", &hgt);
    if let Some(alt) = &m.alt {
        h.meta("twitter:image:alt", alt);
        h.meta("og:image:alt", alt);
    }
}

/// `owoembed` link: Discord shows `text` as the author line and `provider` above the title.
fn oembed_href(page: &Page, lines: &Lines, social: Option<&str>) -> String {
    let post = page.post;
    let engagement = match social {
        Some(s) if post.text.as_deref().is_some_and(|t| !t.trim().is_empty()) => s,
        _ => DEFAULT_AUTHOR_TEXT,
    };
    let gif = match page.index {
        Some(i) => post.media[i].gif,
        None => post.media.iter().find(|m| m.video.is_some()).is_some_and(|m| m.gif),
    };
    let provider = if gif {
        format!("GIF - {}", brand(post.provider))
    } else if lines.card == "player" && engagement != DEFAULT_AUTHOR_TEXT {
        engagement.to_owned()
    } else {
        String::new()
    };
    let mut href = format!(
        "{}/owoembed?text={}&status={}&author={}",
        page.base,
        encode(&truncate(&lines.author_text, 255)),
        encode(&post.id),
        encode(&post.handle),
    );
    if !provider.is_empty() {
        let _ = write!(href, "&provider={}", encode(&provider));
    }
    if let Some(realm) = activity::realm(post.provider) {
        let _ = write!(href, "&realm={realm}");
    }
    href
}

/// FxEmbed's `handleQuote`.
fn quote_text(quote: &Post, translated: Option<&str>) -> String {
    let name = quote.display_name.as_deref().unwrap_or(&quote.handle);
    let text = translated.or(quote.text.as_deref()).unwrap_or_default();
    format!("\nQuoting {name} (@{}) \n\n{text}", quote.handle)
}

/// Telegram Instant View article body (FxEmbed pretends to be Medium to get one).
fn instant_view(page: &Page) -> String {
    let post = page.post;
    let url = escape(&post.url());
    let mut out = format!(
        "<section class=\"section-backgroundImage\"><figure class=\"graf--layoutFillWidth\"></figure></section>\
         <section class=\"section--first\">If you can see this, your browser is doing something weird with your user agent. \
         <a href=\"{url}\">View full thread</a></section>\
         <article><sub><a href=\"{url}\">View full thread</a></sub><h1>{}</h1>",
        escape(&post.title())
    );
    for (n, m) in (1..).zip(&post.media) {
        match &m.video {
            None => {
                let alt = m.alt.as_deref().map(|a| format!(r#" alt="{}""#, escape(a)));
                let _ = write!(
                    out,
                    r#"<img src="{}"{}/>"#,
                    escape(&media_url(page.base, post, n, "image.jpg")),
                    alt.unwrap_or_default()
                );
            }
            Some(_) => {
                let what = if m.gif { "GIF" } else { "video" };
                let author = post.display_name.as_deref().unwrap_or(&post.handle);
                let _ = write!(
                    out,
                    r#"<video src="{}" alt="{}'s {what}. Alt text not available."/>"#,
                    escape(&media_url(page.base, post, n, "video.mp4")),
                    escape(author)
                );
            }
        }
    }
    if let Some(Translated { source, text: Some(translated), .. }) = page.translation {
        let _ = write!(
            out,
            "<h4>📑 Translated from {}</h4>{}<h4>Original text</h4>",
            escape(source),
            paragraphs(&linkify(translated, post.provider), "p")
        );
    }
    out.push_str(&paragraphs(&linkify(post.text.as_deref().unwrap_or_default(), post.provider), "p"));
    if let Some(quote) = &post.quote {
        let _ = write!(
            out,
            "<h4><a href=\"{}\">Quoting</a> {} (<a href=\"{}\">@{}</a>)</h4>{}",
            escape(&quote.url()),
            escape(quote.display_name.as_deref().unwrap_or(&quote.handle)),
            escape(&quote.author_url()),
            escape(&quote.handle),
            paragraphs(
                &linkify(quote.text.as_deref().unwrap_or_default(), quote.provider),
                "blockquote"
            )
        );
    }
    let social: Vec<String> = engagement(post)
        .into_iter()
        .map(|(emoji, n, _)| format!("{emoji} {}", format_number(n)))
        .collect();
    let _ = write!(
        out,
        "<p>{}</p><br><a href=\"{url}\">View full thread</a></article>",
        social.join(" ")
    );
    out
}

/// Wraps each non-empty line of already-escaped `html` in `<tag>`.
fn paragraphs(html: &str, tag: &str) -> String {
    html.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .fold(String::new(), |mut out, line| {
            let _ = write!(out, "<{tag}>{line}</{tag}>");
            out
        })
}

/// Nonzero engagement counters: (emoji, count, X intent URL). Syndication has no repost or
/// view counts, so FxTwitter's 🔁 and 👁️ never appear.
pub fn engagement(post: &Post) -> Vec<(&'static str, u64, Option<String>)> {
    let x = post.provider == Provider::Twitter;
    let id = &post.id;
    [
        ("💬", post.comments, format!("https://x.com/intent/tweet?in_reply_to={id}")),
        ("❤️", post.likes, format!("https://x.com/intent/like?tweet_id={id}")),
    ]
    .into_iter()
    .filter_map(|(emoji, n, intent)| Some((emoji, n.filter(|&n| n > 0)?, x.then_some(intent))))
    .collect()
}

/// FxEmbed's `getSocialProof`: `💬 12   ❤️ 3.4K`.
pub fn social_proof(post: &Post) -> Option<String> {
    let parts: Vec<String> = engagement(post)
        .into_iter()
        .map(|(emoji, n, _)| format!("{emoji} {}", format_number(n)))
        .collect();
    (!parts.is_empty()).then(|| parts.join("   "))
}

/// FxEmbed's `formatNumber`.
pub fn format_number(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}K", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

/// FxEmbed's `owoembed` route.
#[derive(Serialize)]
pub struct OEmbed {
    author_name: String,
    author_url: String,
    provider_name: String,
    provider_url: String,
    title: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    version: &'static str,
}

pub fn owoembed(provider: Provider, text: Option<String>, status: &str, author: &str, provider_text: Option<String>) -> OEmbed {
    let status_url = match provider {
        Provider::Twitter => format!("https://x.com/{}/status/{}", encode(author), encode(status)),
        Provider::Instagram => format!("https://www.instagram.com/p/{}/", encode(status)),
    };
    let provider_url = match provider_text {
        Some(_) => status_url.clone(),
        None => site_origin(provider).to_owned(),
    };
    OEmbed {
        author_name: text.unwrap_or_else(|| brand(provider).to_owned()),
        author_url: status_url,
        provider_name: provider_text.unwrap_or_else(|| brand(provider).to_owned()),
        provider_url,
        title: DEFAULT_AUTHOR_TEXT,
        kind: "rich",
        version: "1.0",
    }
}

/// Escapes `text` and links URLs, `@mentions` and `#hashtags` (FxEmbed's `formatStatus`).
/// Newlines are kept.
pub fn linkify(text: &str, provider: Provider) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let at_boundary = prev.is_none_or(|p| !(p.is_alphanumeric() || p == '_'));
        let token = if !at_boundary {
            None
        } else if let Some(len) = url_len(rest) {
            let url = escape(&rest[..len]);
            Some((len, format!(r#"<a href="{url}">{url}</a>"#)))
        } else if c == '@' {
            let len = handle_len(&rest[1..], provider);
            (len > 0).then(|| {
                let handle = &rest[1..=len];
                let href = match provider {
                    Provider::Twitter => format!("https://x.com/{handle}"),
                    Provider::Instagram => format!("https://www.instagram.com/{handle}/"),
                };
                (len + 1, format!(r#"<a href="{href}">@{handle}</a>"#))
            })
        } else if c == '#' {
            let len = rest[1..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len() - 1);
            let tag = &rest[1..=len];
            // X hashtags need a non-digit.
            (!tag.chars().all(|c| c.is_ascii_digit())).then(|| {
                let tag = escape(tag);
                let href = match provider {
                    Provider::Twitter => format!("https://x.com/hashtag/{tag}"),
                    Provider::Instagram => format!("https://www.instagram.com/explore/tags/{tag}/"),
                };
                (len + 1, format!(r##"<a href="{href}">#{tag}</a>"##))
            })
        } else {
            None
        };
        match token {
            Some((len, html)) => {
                out.push_str(&html);
                prev = rest[..len].chars().next_back();
                rest = &rest[len..];
            }
            None => {
                out.push_str(&escape(c.encode_utf8(&mut [0; 4])));
                prev = Some(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

fn url_len(s: &str) -> Option<usize> {
    let scheme = ["https://", "http://"].into_iter().find(|p| s.starts_with(p))?;
    let end = s
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .unwrap_or(s.len());
    let mut url = &s[..end];
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ':', ';', '!', '?', '\'', '"']);
        let trimmed = match trimmed.strip_suffix(')') {
            Some(t) if trimmed.matches('(').count() < trimmed.matches(')').count() => t,
            _ => trimmed,
        };
        if trimmed.len() == url.len() {
            break;
        }
        url = trimmed;
    }
    (url.len() > scheme.len()).then_some(url.len())
}

fn handle_len(s: &str, provider: Provider) -> usize {
    let len = s
        .find(|c: char| {
            !(c.is_ascii_alphanumeric() || c == '_' || (provider == Provider::Instagram && c == '.'))
        })
        .unwrap_or(s.len());
    s[..len].trim_end_matches('.').len()
}

/// Truncates to `max` chars with an ellipsis (FxEmbed's `truncateWithEllipsis`).
pub fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_owned(),
    }
}

/// URL query/path component encoding.
pub fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISCORD: &str = "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)";
    const TELEGRAM: &str = "TelegramBot (like TwitterBot)";
    const SLACK: &str = "Slackbot-LinkExpanding 1.0 (+https://api.slack.com/robots)";

    fn photo(n: u32) -> Media {
        Media {
            video: None,
            image: format!("https://pbs.twimg.com/media/p{n}.jpg"),
            width: 1200,
            height: 800,
            alt: None,
            gif: false,
        }
    }

    fn video() -> Media {
        Media {
            video: Some("https://video.twimg.com/v.mp4".into()),
            image: "https://pbs.twimg.com/thumb.jpg".into(),
            width: 1280,
            height: 720,
            alt: None,
            gif: false,
        }
    }

    fn tweet(media: Vec<Media>) -> Post {
        Post {
            provider: Provider::Twitter,
            id: "20".into(),
            handle: "jack".into(),
            display_name: Some("jack".into()),
            text: Some("just setting up my twttr".into()),
            lang: Some("en".into()),
            created_at: Some("2006-03-21T20:50:14.000Z".into()),
            avatar: Some("https://pbs.twimg.com/profile_images/a_200x200.jpg".into()),
            reply_to: None,
            likes: Some(1234),
            comments: Some(5),
            media,
            quote: None,
        }
    }

    fn render(post: &Post, ua: &str, label: &str, index: Option<usize>) -> String {
        render_html(&Page {
            post,
            flags: Flags::new(label, false),
            agent: Agent::new(ua),
            index,
            translation: None,
            lang: None,
            base: "https://fix.test",
            origin: "https://fix.test",
        })
    }

    fn count(html: &str, needle: &str) -> usize {
        html.matches(needle).count()
    }

    #[test]
    fn detects_unfurlers_not_browsers() {
        assert!(is_bot(DISCORD));
        assert!(is_bot(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.10; rv:38.0) Gecko/20100101 Firefox/38.0"
        ));
        assert!(is_bot(TELEGRAM));
        assert!(is_bot("WhatsApp/2.23.20.0"));
        assert!(!is_bot(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
        ));
        assert!(!is_bot(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
        ));
        assert!(!is_bot(""));
    }

    #[test]
    fn parses_modifier_subdomains() {
        assert_eq!(host_label("fix.test", "fix.test"), Some(""));
        assert_eq!(host_label("fix.test:8080", "fix.test"), Some(""));
        assert_eq!(host_label("D.Fix.Test", "fix.test"), Some("d"));
        assert_eq!(host_label("dl.fix.test:443", "fix.test"), Some("dl"));
        assert_eq!(host_label("x.fix.test", "fix.test"), None);
        assert_eq!(host_label("d.evil.test", "fix.test"), None);
        assert_eq!(host_label("[::1]:8080", "[::1]"), Some(""));
        assert_eq!(Flags::new("g", false), Flags { gallery: true, ..Flags::default() });
        // A `.mp4` path wins over the subdomain.
        assert_eq!(Flags::new("t", true), Flags { direct: true, ..Flags::default() });
    }

    #[test]
    fn discord_gets_activity_link_unless_opted_out() {
        let post = tweet(vec![photo(1), photo(2)]);
        let html = render(&post, DISCORD, "", None);
        assert_eq!(count(&html, "application/activity+json"), 1);
        assert_eq!(count(&html, r#"property="og:image""#), 0, "activity embeds carry the media");
        let html = render(&post, DISCORD, "o", None);
        assert_eq!(count(&html, "application/activity+json"), 0);
        // Discord shows every photo natively; the mosaic is for everyone else.
        assert_eq!(count(&html, r#"property="og:image""#), 2);
        assert!(!html.contains("/mosaic/"));
        // `m.` keeps the activity embed; the snowcode tells it to send the mosaic.
        let html = render(&post, DISCORD, "m", None);
        assert_eq!(count(&html, "application/activity+json"), 1);
        let html = render(&post, "matrixpreviewbot", "m", None);
        assert_eq!(count(&html, r#"property="og:image""#), 1);
        assert!(html.contains("https://fix.test/mosaic/x/20.jpg"));
        let html = render(&post, SLACK, "", None);
        assert!(html.contains("https://fix.test/mosaic/x/20.jpg"));
    }

    #[test]
    fn photo_number_overrides_mosaic_with_counter() {
        let post = tweet(vec![photo(1), photo(2), photo(3)]);
        let html = render(&post, SLACK, "", Some(1));
        assert!(html.contains("https://fix.test/media/x/20/2/image.jpg"));
        assert!(!html.contains("/mosaic/"));
        assert!(html.contains(r#"content="X - 💬 5   ❤️ 1.2K - Photo 2 / 3""#));
    }

    #[test]
    fn gallery_and_text_only_strip_parts() {
        let post = tweet(vec![video()]);
        let html = render(&post, SLACK, "g", None);
        assert!(html.contains("og:video"));
        assert!(!html.contains("og:description"));
        assert!(!html.contains("json+oembed"));
        assert!(!html.contains("theme-color"));
        let html = render(&post, SLACK, "t", None);
        assert!(!html.contains("og:video"));
        assert!(!html.contains("og:image"));
        assert!(html.contains("og:description"));
    }

    #[test]
    fn video_moves_text_to_author_line() {
        let post = tweet(vec![video()]);
        let html = render(&post, SLACK, "", None);
        assert!(html.contains("text=just+setting+up+my+twttr"));
        assert!(html.contains("provider=%F0%9F%92%AC+5+++%E2%9D%A4%EF%B8%8F+1.2K"));
        assert_eq!(count(&html, r#"content="1280""#), 2);
    }

    #[test]
    fn text_only_post_uses_avatar_and_reply_author_line() {
        let mut post = tweet(vec![]);
        post.likes = None;
        post.comments = None;
        post.reply_to = Some("biz".into());
        let html = render(&post, SLACK, "", None);
        assert!(html.contains(r#"<meta property="og:image" content="https://pbs.twimg.com/profile_images/a_200x200.jpg"/>"#));
        assert!(html.contains(r#"<meta property="twitter:image" content="0"/>"#));
        assert!(html.contains("text=%E2%86%AA+Replying+to+%40biz"));
        assert!(html.contains("↩ Replying to @biz\njust setting up"));
    }

    #[test]
    fn telegram_gets_instant_view_for_photos() {
        let post = tweet(vec![photo(1)]);
        let html = render(&post, TELEGRAM, "", None);
        assert!(html.contains(r#"content="Medium""#));
        assert!(html.contains("<article>"));
        assert!(!html.contains("http-equiv"));
        let html = render(&tweet(vec![video()]), TELEGRAM, "", None);
        assert!(!html.contains("<article>"));
    }

    #[test]
    fn linkifies_like_fxembed() {
        assert_eq!(
            linkify("hi @jack, see https://a.test/x?y=1&z=2. #tag a@b #123\n<b>", Provider::Twitter),
            "hi <a href=\"https://x.com/jack\">@jack</a>, see \
             <a href=\"https://a.test/x?y=1&amp;z=2\">https://a.test/x?y=1&amp;z=2</a>. \
             <a href=\"https://x.com/hashtag/tag\">#tag</a> a@b #123\n&lt;b&gt;"
        );
        assert_eq!(
            linkify("@first.last. (https://w.test/a_(b))", Provider::Instagram),
            "<a href=\"https://www.instagram.com/first.last/\">@first.last</a>. \
             (<a href=\"https://w.test/a_(b)\">https://w.test/a_(b)</a>)"
        );
    }

    #[test]
    fn formats_numbers_like_fxembed() {
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_260), "1.3K");
        assert_eq!(format_number(1_234_567), "1.23M");
    }
}
