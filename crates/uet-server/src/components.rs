//! Discord component embeds: a `<script id="discord:component-embed">` JSON payload in the
//! page head. Discord renders it as a Components V2 container instead of the normal link
//! preview, and falls back to the OpenGraph tags when it can't.
//!
//! Spec: <https://docs.discord.com/developers/link-previews/component-embeds>. The payload is
//! one Container (17) holding Section (9), Text Display (10), Thumbnail (11), Media Gallery
//! (12), Separator (14), Action Row (1) and link Buttons (2, style 5). It may be at most
//! 3,000 bytes, `<`-style escapes counted at full length, with at most 10 gallery items.

use serde_json::{Value, json};
use uet_core::Provider;

use crate::{
    embed::{Page, brand, engagement, format_number, media_url, mosaic_url, truncate, url_len},
    translate::Translated,
};

pub const MAX_BYTES: usize = 3000;
pub const MAX_GALLERY_ITEMS: usize = 10;
/// Discord's limit on gallery item descriptions.
const MAX_ALT_CHARS: usize = 1024;
/// Text is cut to this length before the avatar is dropped to make room.
const SHORT_TEXT_CHARS: usize = 280;

const CONTAINER: u8 = 17;
const SECTION: u8 = 9;
const TEXT: u8 = 10;
const THUMBNAIL: u8 = 11;
const GALLERY: u8 = 12;
const ACTION_ROW: u8 = 1;
const BUTTON: u8 = 2;
const LINK_STYLE: u8 = 5;

/// What to leave out when the payload is over [`MAX_BYTES`].
#[derive(Clone, Copy)]
struct Trim {
    /// Max chars of the post text and of the quote text; `None` keeps them whole.
    text_chars: Option<usize>,
    alt: bool,
    avatar: bool,
    items: usize,
}

pub fn avatar_url(page: &Page) -> String {
    let post = page.post;
    format!(
        "{}/media/{}/{}/avatar.jpg",
        page.base,
        post.provider.slug(),
        post.id
    )
}

/// The `<script>` element, or `None` if even the smallest payload is over the limit.
pub fn script_tag(page: &Page) -> Option<String> {
    let json = fit(page)?;
    Some(format!(
        "<script id=\"discord:component-embed\" type=\"application/json\">{json}</script>\n"
    ))
}

/// Serialized payload that fits in [`MAX_BYTES`], cutting the least useful parts first:
/// alt text, then long text, then the avatar, then the rest of the text, then media items.
pub fn fit(page: &Page) -> Option<String> {
    let mut trim = Trim {
        text_chars: None,
        alt: true,
        avatar: true,
        items: MAX_GALLERY_ITEMS,
    };
    let mut json = serialize(&payload(page, trim));
    if json.len() <= MAX_BYTES {
        return Some(json);
    }
    trim.alt = false;
    json = serialize(&payload(page, trim));

    let longest = longest_text_chars(page);
    for floor in [SHORT_TEXT_CHARS, 0] {
        if floor == 0 {
            trim.avatar = false;
            json = serialize(&payload(page, trim));
        }
        // Each pass cuts at least as many chars as there are bytes too many. A char is at
        // least one byte, so this ends after a few passes.
        while json.len() > MAX_BYTES {
            let current = trim.text_chars.unwrap_or(longest);
            if current <= floor {
                break;
            }
            let over = json.len() - MAX_BYTES;
            trim.text_chars = Some(current.saturating_sub(over + 8).max(floor));
            json = serialize(&payload(page, trim));
        }
        if json.len() <= MAX_BYTES {
            return Some(json);
        }
    }
    while json.len() > MAX_BYTES && trim.items > 0 {
        trim.items -= 1;
        json = serialize(&payload(page, trim));
    }
    (json.len() <= MAX_BYTES).then_some(json)
}

fn longest_text_chars(page: &Page) -> usize {
    let quote = page.post.quote.as_ref().and_then(|q| q.text.as_deref());
    [body_text(page), quote]
        .into_iter()
        .flatten()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(0)
}

/// JSON safe to put inside `<script>`: `<` is escaped so the text can't close the element.
fn serialize(value: &Value) -> String {
    value.to_string().replace('<', "\\u003c")
}

/// Post text, or its translation.
fn body_text<'a>(page: &'a Page) -> Option<&'a str> {
    match page.translation {
        Some(Translated { text: Some(t), .. }) => Some(t.as_str()),
        _ => page.post.text.as_deref(),
    }
    .filter(|t| !t.trim().is_empty())
}

fn cut(text: &str, trim: Trim) -> String {
    match trim.text_chars {
        Some(n) => truncate(text.trim(), n),
        None => text.trim().to_owned(),
    }
}

fn payload(page: &Page, trim: Trim) -> Value {
    let post = page.post;
    let flags = page.flags;
    let gallery_only = flags.gallery && !post.media.is_empty();
    let mut parts: Vec<Value> = Vec::new();

    // Author line and text, with the avatar beside them.
    let mut head = format!(
        "**[{}]({})**",
        escape_markdown(&post.title()),
        post.author_url()
    );
    if !gallery_only {
        if let Some(to) = &post.reply_to {
            head.push_str(&format!("\n-# ↪ Replying to @{}", escape_markdown(to)));
        }
        if let Some(Translated {
            source,
            text: Some(_),
            ..
        }) = page.translation
        {
            head.push_str(&format!(
                "\n-# 📑 Translated from {}",
                escape_markdown(source)
            ));
        }
        if let Some(text) = body_text(page) {
            head.push('\n');
            head.push_str(&escape_markdown(&cut(text, trim)));
        }
    }
    let head = json!({ "type": TEXT, "content": head });
    parts.push(if trim.avatar && post.avatar.is_some() {
        json!({
            "type": SECTION,
            "components": [head],
            "accessory": { "type": THUMBNAIL, "media": { "url": avatar_url(page) } },
        })
    } else {
        head
    });

    if !gallery_only && let Some(quote) = &post.quote {
        let translated = page.translation.and_then(|t| t.quote.as_deref());
        let mut q = format!("**[{}]({})**", escape_markdown(&quote.title()), quote.url());
        if let Some(text) = translated
            .or(quote.text.as_deref())
            .filter(|t| !t.trim().is_empty())
        {
            q.push('\n');
            q.push_str(&escape_markdown(&cut(text, trim)));
        }
        let quoted: Vec<String> = q.lines().map(|l| format!("> {l}")).collect();
        parts.push(json!({ "type": TEXT, "content": quoted.join("\n") }));
    }

    if !flags.text_only {
        let items = gallery_items(page, trim);
        if !items.is_empty() {
            parts.push(json!({ "type": GALLERY, "items": items }));
        }
    }

    if !gallery_only {
        let mut footer: Vec<String> = engagement(post)
            .into_iter()
            .map(|(emoji, n, _)| format!("{emoji} {}", format_number(n)))
            .collect();
        footer.push(brand(post.provider).to_owned());
        parts.push(json!({ "type": TEXT, "content": format!("-# {}", footer.join("   ")) }));
    }

    parts.push(json!({
        "type": ACTION_ROW,
        "components": [{
            "type": BUTTON,
            "style": LINK_STYLE,
            "label": format!("Open on {}", brand(post.provider)),
            "url": post.url(),
        }],
    }));

    json!({
        "component": {
            "type": CONTAINER,
            "accent_color": accent_color(post.provider),
            "components": parts,
        }
    })
}

fn gallery_items(page: &Page, trim: Trim) -> Vec<Value> {
    let post = page.post;
    let item = |url: String, alt: Option<&str>| {
        let mut v = json!({ "media": { "url": url } });
        if trim.alt
            && let Some(alt) = alt.filter(|a| !a.trim().is_empty())
        {
            v["description"] = json!(truncate(alt, MAX_ALT_CHARS));
        }
        v
    };
    let one = |i: usize| {
        let m = &post.media[i];
        let file = if m.video.is_some() {
            "video.mp4"
        } else {
            "image.jpg"
        };
        item(media_url(page.base, post, i + 1, file), m.alt.as_deref())
    };
    let mut items: Vec<Value> = match page.index {
        Some(i) => vec![one(i)],
        // `m.`: photos become one mosaic image, videos stay as they are.
        None if page.flags.force_mosaic && post.mosaic_photos().is_some() => {
            let mut out = vec![item(mosaic_url(page.base, post), None)];
            out.extend(
                (0..post.media.len())
                    .filter(|&i| post.media[i].video.is_some())
                    .map(one),
            );
            out
        }
        None => (0..post.media.len()).map(one).collect(),
    };
    items.truncate(trim.items);
    items
}

fn accent_color(provider: Provider) -> u32 {
    match provider {
        Provider::Instagram => 0xE1_30_6C,
        Provider::Twitter => 0x1D_9B_F0,
    }
}

/// Backslash-escapes Discord markdown so post text shows as written. URLs are left alone so
/// Discord still links them. `#`, `-` and `>` only mean something at the start of a line, and
/// a hashtag is not a heading (that needs a space after `#`), so only those are escaped there.
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let body = line.trim_start();
        out.push_str(&line[..line.len() - body.len()]);
        if body.starts_with(['>', '-'])
            || body.starts_with("# ")
            || body.starts_with("## ")
            || body.starts_with("### ")
        {
            out.push('\\');
        }
        let mut rest = body;
        while let Some(c) = rest.chars().next() {
            if let Some(len) = url_len(rest) {
                out.push_str(&rest[..len]);
                rest = &rest[len..];
                continue;
            }
            if matches!(
                c,
                '\\' | '*' | '_' | '~' | '`' | '|' | '[' | ']' | '<' | ':'
            ) {
                out.push('\\');
            }
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::DiscordEmbed,
        embed::{Agent, Flags},
        post::{Media, Post},
    };

    const DISCORD: &str = "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)";

    fn media(n: usize, video: bool) -> Media {
        Media {
            video: video.then(|| format!("https://cdn.test/{n}.mp4")),
            image: format!("https://cdn.test/{n}.jpg"),
            width: 1080,
            height: 1350,
            alt: Some(format!("Photo by someone, item {n}")),
            gif: false,
        }
    }

    fn reel(items: Vec<Media>, text: &str) -> Post {
        Post {
            provider: Provider::Instagram,
            id: "DeMPFloOrjv".into(),
            handle: "albertheijn".into(),
            display_name: Some("Albert Heijn".into()),
            text: Some(text.into()),
            lang: None,
            created_at: None,
            avatar: Some(format!(
                "https://scontent.cdninstagram.com/v/{}",
                "x".repeat(700)
            )),
            reply_to: None,
            likes: Some(51_800),
            comments: Some(358),
            media: items,
            quote: None,
        }
    }

    fn page<'a>(post: &'a Post, label: &str, index: Option<usize>) -> Page<'a> {
        Page {
            post,
            flags: Flags::new(label, false),
            agent: Agent::new(DISCORD),
            index,
            translation: None,
            lang: None,
            base: "https://fix.test",
            origin: "https://fix.test",
            discord_embed: DiscordEmbed::Components,
        }
    }

    fn parsed(page: &Page) -> Value {
        let json = fit(page).expect("fits");
        assert!(json.len() <= MAX_BYTES, "{} bytes", json.len());
        serde_json::from_str(&json).unwrap()
    }

    /// Every component in the payload, depth first.
    fn walk(v: &Value, out: &mut Vec<Value>) {
        out.push(v.clone());
        if let Some(children) = v["components"].as_array() {
            children.iter().for_each(|c| walk(c, out));
        }
        if v["accessory"].is_object() {
            walk(&v["accessory"], out);
        }
    }

    fn components(v: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        walk(&v["component"], &mut out);
        out
    }

    fn gallery_urls(v: &Value) -> Vec<String> {
        components(v)
            .iter()
            .filter(|c| c["type"] == GALLERY)
            .flat_map(|g| g["items"].as_array().cloned().unwrap_or_default())
            .map(|i| i["media"]["url"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn builds_a_valid_container() {
        let post = reel(
            vec![media(1, true)],
            "Jouw beurt. Kan jij deze PR verbeteren? 🏃💨",
        );
        let v = parsed(&page(&post, "", None));
        let all = components(&v);
        assert_eq!(v["component"]["type"], CONTAINER);
        assert_eq!(all.iter().filter(|c| c["type"] == CONTAINER).count(), 1);
        assert!(all.len() <= 40);
        assert_eq!(
            gallery_urls(&v),
            ["https://fix.test/media/ig/DeMPFloOrjv/1/video.mp4"]
        );
        let thumb = all.iter().find(|c| c["type"] == THUMBNAIL).unwrap();
        assert_eq!(
            thumb["media"]["url"],
            "https://fix.test/media/ig/DeMPFloOrjv/avatar.jpg"
        );
        let button = all.iter().find(|c| c["type"] == BUTTON).unwrap();
        assert_eq!(button["style"], LINK_STYLE);
        assert_eq!(button["url"], "https://www.instagram.com/p/DeMPFloOrjv/");
        // Link buttons may only carry these keys.
        let keys: Vec<&String> = button.as_object().unwrap().keys().collect();
        assert!(keys.iter().all(|k| {
            ["id", "type", "url", "style", "label", "emoji", "disabled"].contains(&k.as_str())
        }));
        let text = &all.iter().find(|c| c["type"] == TEXT).unwrap()["content"];
        assert!(text.as_str().unwrap().contains("Jouw beurt."));
        assert!(
            serde_json::to_string(&v)
                .unwrap()
                .contains("💬 358   ❤️ 51.8K   Instagram")
        );
    }

    #[test]
    fn caps_gallery_at_ten_and_honours_index_and_text_only() {
        let post = reel((1..=12).map(|n| media(n, n % 3 == 0)).collect(), "carousel");
        let v = parsed(&page(&post, "", None));
        let urls = gallery_urls(&v);
        assert_eq!(urls.len(), 10);
        assert_eq!(urls[2], "https://fix.test/media/ig/DeMPFloOrjv/3/video.mp4");
        let v = parsed(&page(&post, "", Some(4)));
        assert_eq!(
            gallery_urls(&v),
            ["https://fix.test/media/ig/DeMPFloOrjv/5/image.jpg"]
        );
        let v = parsed(&page(&post, "t", None));
        assert!(gallery_urls(&v).is_empty());
        let v = parsed(&page(&post, "m", None));
        assert_eq!(
            gallery_urls(&v)[0],
            "https://fix.test/mosaic/ig/DeMPFloOrjv.jpg"
        );
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        let long = "Lange tekst met <script> en </script> erin. ".repeat(200);
        let post = reel((1..=10).map(|n| media(n, false)).collect(), &long);
        let json = fit(&page(&post, "", None)).unwrap();
        assert!(json.len() <= MAX_BYTES);
        assert!(!json.contains("</script"));
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(gallery_urls(&v).len(), 10, "media is kept, text is cut");
        assert!(json.contains('…'));
    }

    #[test]
    fn gallery_modifier_drops_text_and_counts() {
        let post = reel(vec![media(1, false), media(2, false)], "caption");
        let json = fit(&page(&post, "g", None)).unwrap();
        assert!(!json.contains("caption"));
        assert!(!json.contains("💬"));
        assert!(json.contains("Open on Instagram"));
    }

    #[test]
    fn escapes_markdown_but_not_urls() {
        assert_eq!(
            escape_markdown("**bold** _x_ see https://a.test/a_b_c?d=*e*\n# Title\n#tag\n- item"),
            "\\*\\*bold\\*\\* \\_x\\_ see https://a.test/a_b_c?d=*e*\n\\# Title\n#tag\n\\- item"
        );
        assert_eq!(escape_markdown("<t:0:R> <@123>"), "\\<t\\:0\\:R> \\<@123>");
    }
}
