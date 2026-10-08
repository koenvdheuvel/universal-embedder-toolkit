//! Instagram and X/Twitter URL recognition and rewriting, shared by the server and the clipboard tool.
//!
//! The fix host mirrors the original sites' path layouts, so rewriting a link is a host swap
//! plus dropping tracking parameters. Instagram and X paths don't overlap, so one host serves both.

use std::num::NonZeroU32;

use url::Url;

/// Hosts that serve Instagram post pages.
pub const INSTAGRAM_HOSTS: &[&str] = &[
    "instagram.com",
    "www.instagram.com",
    "m.instagram.com",
    "instagr.am",
    "www.instagr.am",
];

/// Hosts that serve X/Twitter status pages.
pub const TWITTER_HOSTS: &[&str] = &[
    "x.com",
    "www.x.com",
    "mobile.x.com",
    "twitter.com",
    "www.twitter.com",
    "mobile.twitter.com",
];

/// Path segments that introduce an Instagram shortcode.
const INSTAGRAM_KINDS: &[&str] = &["p", "reel", "reels", "tv"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Instagram,
    Twitter,
}

impl Provider {
    /// Short URL slug used in the server's own routes.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Instagram => "ig",
            Self::Twitter => "x",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "ig" => Some(Self::Instagram),
            "x" => Some(Self::Twitter),
            _ => None,
        }
    }

    /// Whether `id` is a well-formed post id (shortcode / tweet id) for this provider.
    pub fn is_valid_id(self, id: &str) -> bool {
        match self {
            Self::Instagram => is_shortcode(id),
            Self::Twitter => is_tweet_id(id),
        }
    }
}

/// What a fix-host path points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    Post {
        provider: Provider,
        id: &'a str,
        /// X screen name from the path (`/{user}/status/...`); `None` for `/i/web/status/...`
        /// and on Instagram.
        handle: Option<&'a str>,
        /// 1-based media index (`?img_index=N` on Instagram, `/photo/N` or `/video/N` on X).
        index: Option<NonZeroU32>,
        mods: PathMods<'a>,
    },
    /// Instagram `/share/...` short link; Instagram redirects it to a post.
    InstagramShare,
}

/// FxTwitter-style modifiers carried in the path itself (host-prefix flags are the server's job).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathMods<'a> {
    /// `{id}.mp4` / `.jpg` / `.jpeg` / `.png` / `.gif` / `.gifv`, or a `/dl/` / `/dir/` prefix:
    /// link straight to the media file.
    pub direct: bool,
    /// Image size after the extension (`{id}.jpg:orig`).
    pub image_name: Option<&'a str>,
    /// Segment after the id or media index (`/status/{id}/ja`), unvalidated; meant as a
    /// translation target.
    pub lang: Option<&'a str>,
}

/// Classifies a path on the fix host, trying X first (its `/{user}/status/{digits}` shape is the
/// more specific one).
pub fn classify_path<'a>(path: &'a str, query: Option<&str>) -> Option<Target<'a>> {
    classify_twitter(path).or_else(|| classify_instagram(path, query))
}

/// `/reel/ABC/`, `/user/p/ABC/`, `/share/reel/xyz/`, plus `/p/ABC.mp4` and `/p/ABC/{lang}`.
pub fn classify_instagram<'a>(path: &'a str, query: Option<&str>) -> Option<Target<'a>> {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    let first = segs.next()?;
    let second = segs.next()?;
    if first == "share" {
        return Some(Target::InstagramShare);
    }
    let id_seg = if INSTAGRAM_KINDS.contains(&first) {
        second
    } else if INSTAGRAM_KINDS.contains(&second) {
        segs.next()?
    } else {
        return None;
    };
    let (id, suffix) = id_seg.split_at(id_seg.find('.').unwrap_or(id_seg.len()));
    let mut mods = media_suffix(suffix)?;
    if !is_shortcode(id) {
        return None;
    }
    mods.lang = segs.next();
    Some(Target::Post {
        provider: Provider::Instagram,
        id,
        handle: None,
        index: image_index(query),
        mods,
    })
}

/// FxTwitter's status routes: `[/dl|/dir]/{user}/{status|statuses|article}/{id}`, `/status/{id}`,
/// `/i/web/status/{id}`, optionally followed by `/photo/N` or `/video/N` (N = 1-4) and `/{lang}`.
pub fn classify_twitter(path: &str) -> Option<Target<'_>> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let is_kind = |s: &&str| matches!(*s, "status" | "statuses" | "article");
    let (direct_prefix, rest) = match segs.split_first()? {
        (&("dl" | "dir"), rest) if rest.len() >= 3 && is_kind(&rest[1]) => (true, rest),
        _ => (false, &segs[..]),
    };
    let (handle, after_kind) = match rest {
        ["i", "web", kind, tail @ ..] if is_kind(kind) => (None, tail),
        [user, kind, tail @ ..] if is_kind(kind) && !tail.is_empty() => {
            (Some(*user).filter(|u| *u != "i"), tail)
        }
        [kind, tail @ ..] if is_kind(kind) => (None, tail),
        _ => return None,
    };
    let (id_seg, tail) = after_kind.split_first()?;
    let digits = id_seg.bytes().take_while(u8::is_ascii_digit).count();
    let (id, suffix) = id_seg.split_at(digits);
    if !is_tweet_id(id) {
        return None;
    }
    // Anything other than a media extension (e.g. Discord's spoiler `||`) is ignored.
    let mut mods = media_suffix(suffix).unwrap_or_default();
    mods.direct |= direct_prefix;
    let (index, tail) = match tail {
        [kind, n, tail @ ..] if matches!(*kind, "photo" | "photos" | "video" | "videos") => {
            (n.parse().ok().filter(|n: &NonZeroU32| n.get() <= 4), tail)
        }
        _ => (None, tail),
    };
    mods.lang = tail.first().copied();
    Some(Target::Post {
        provider: Provider::Twitter,
        id,
        handle,
        index,
        mods,
    })
}

/// Parses what follows a post id: empty, or `.mp4` / `.jpg:orig` etc. `None` for anything else.
fn media_suffix(suffix: &str) -> Option<PathMods<'_>> {
    let Some(ext) = suffix.strip_prefix('.') else {
        return suffix.is_empty().then(PathMods::default);
    };
    let (ext, image_name) = match ext.split_once(':') {
        Some((ext, name)) => (ext, Some(name).filter(|n| !n.is_empty())),
        None => (ext, None),
    };
    const EXTS: &[&str] = &["mp4", "jpg", "jpeg", "png", "gif", "gifv"];
    EXTS.iter()
        .any(|e| ext.eq_ignore_ascii_case(e))
        .then_some(PathMods {
            direct: true,
            image_name,
            lang: None,
        })
}

/// Instagram shortcodes are URL-safe base64; `audio` is the reels audio page, not a post.
pub fn is_shortcode(code: &str) -> bool {
    (1..=64).contains(&code.len())
        && code != "audio"
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Tweet ids are decimal snowflakes (u64).
pub fn is_tweet_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// 1-based carousel index from Instagram's `img_index` query parameter.
pub fn image_index(query: Option<&str>) -> Option<NonZeroU32> {
    url::form_urlencoded::parse(query?.as_bytes())
        .find(|(k, _)| k == "img_index")
        .and_then(|(_, v)| v.parse().ok())
}

fn host_in(host: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|h| host.eq_ignore_ascii_case(h))
}

/// Rewrites a single Instagram or X post URL to the fix host.
///
/// `fix_base` is the public base URL of the server, e.g. `https://fix.example.com`.
/// Returns `None` for anything that is not a supported post link.
pub fn rewrite_url(input: &str, fix_base: &Url) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    let index = if host_in(host, INSTAGRAM_HOSTS) {
        match classify_instagram(url.path(), url.query())? {
            Target::Post { index, .. } => index,
            Target::InstagramShare => None,
        }
    } else if host_in(host, TWITTER_HOSTS) {
        classify_twitter(url.path())?;
        // X's index lives in the path; its query is only tracking (`?s=20&t=...`).
        None
    } else {
        return None;
    };
    let mut out = fix_base.clone();
    out.set_path(url.path());
    out.set_fragment(None);
    out.set_query(None);
    if let Some(index) = index {
        out.query_pairs_mut()
            .append_pair("img_index", &index.to_string());
    }
    Some(out.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(input: &str) -> Option<String> {
        rewrite_url(input, &Url::parse("https://fix.example.com").unwrap())
    }

    fn post(provider: Provider, id: &str, index: Option<u32>) -> Option<Target<'_>> {
        with_mods(provider, id, index, PathMods::default())
    }

    fn with_mods<'a>(
        provider: Provider,
        id: &'a str,
        index: Option<u32>,
        mods: PathMods<'a>,
    ) -> Option<Target<'a>> {
        Some(Target::Post {
            provider,
            id,
            handle: None,
            index: index.and_then(NonZeroU32::new),
            mods,
        })
    }

    /// `classify_path` with the handle cleared; handles are checked in their own test.
    fn classify_path<'a>(path: &'a str, query: Option<&str>) -> Option<Target<'a>> {
        super::classify_path(path, query).map(|t| match t {
            Target::Post { provider, id, index, mods, .. } => Target::Post {
                provider,
                id,
                handle: None,
                index,
                mods,
            },
            share => share,
        })
    }

    #[test]
    fn extracts_x_handle() {
        let handle = |path| match super::classify_path(path, None) {
            Some(Target::Post { handle, .. }) => handle,
            other => panic!("{path}: {other:?}"),
        };
        assert_eq!(handle("/jack/status/20"), Some("jack"));
        assert_eq!(handle("/dl/jack/status/20.mp4"), Some("jack"));
        assert_eq!(handle("/i/status/20"), None);
        assert_eq!(handle("/i/web/status/20"), None);
        assert_eq!(handle("/status/20"), None);
        assert_eq!(handle("/someuser/p/ABC/"), None);
    }

    #[test]
    fn rewrites_instagram_and_drops_tracking() {
        assert_eq!(
            rw("https://www.instagram.com/reels/DeMPFloOrjv/").as_deref(),
            Some("https://fix.example.com/reels/DeMPFloOrjv/")
        );
        assert_eq!(
            rw("  https://instagram.com/reel/DeMPFloOrjv/?igsh=MWx2bTQ1#x \n").as_deref(),
            Some("https://fix.example.com/reel/DeMPFloOrjv/")
        );
        assert_eq!(
            rw("http://M.Instagram.com/tv/AB_c-1").as_deref(),
            Some("https://fix.example.com/tv/AB_c-1")
        );
        assert_eq!(
            rw("https://www.instagram.com/someuser/p/ABC123/").as_deref(),
            Some("https://fix.example.com/someuser/p/ABC123/")
        );
        assert_eq!(
            rw("https://instagr.am/share/reel/BAxyz/").as_deref(),
            Some("https://fix.example.com/share/reel/BAxyz/")
        );
    }

    #[test]
    fn keeps_only_valid_carousel_index() {
        assert_eq!(
            rw("https://www.instagram.com/p/ABC/?igsh=x&img_index=3").as_deref(),
            Some("https://fix.example.com/p/ABC/?img_index=3")
        );
        assert_eq!(
            rw("https://www.instagram.com/p/ABC/?img_index=0").as_deref(),
            Some("https://fix.example.com/p/ABC/")
        );
        assert_eq!(
            rw("https://www.instagram.com/p/ABC/?img_index=two").as_deref(),
            Some("https://fix.example.com/p/ABC/")
        );
    }

    #[test]
    fn rewrites_twitter_and_drops_tracking() {
        assert_eq!(
            rw("https://x.com/elonmusk/status/1585341984679469056?s=46&t=abc").as_deref(),
            Some("https://fix.example.com/elonmusk/status/1585341984679469056")
        );
        assert_eq!(
            rw("https://Mobile.Twitter.com/nasa/status/2050014959552114958/photo/3").as_deref(),
            Some("https://fix.example.com/nasa/status/2050014959552114958/photo/3")
        );
        assert_eq!(
            rw("https://twitter.com/i/web/status/20").as_deref(),
            Some("https://fix.example.com/i/web/status/20")
        );
    }

    #[test]
    fn classifies_fix_host_paths() {
        use Provider::*;
        assert_eq!(classify_path("/jack/status/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/i/status/20/", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/i/web/status/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/status/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/jack/statuses/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/jack/article/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/status/status/20", None), post(Twitter, "20", None));
        assert_eq!(classify_path("/nasa/status/9/photo/2", None), post(Twitter, "9", Some(2)));
        assert_eq!(classify_path("/nasa/status/9/videos/1", None), post(Twitter, "9", Some(1)));
        // FxTwitter only accepts media numbers 1-4.
        assert_eq!(classify_path("/nasa/status/9/photo/0", None), post(Twitter, "9", None));
        assert_eq!(classify_path("/nasa/status/9/photo/5", None), post(Twitter, "9", None));
        // Discord spoiler links append `||`.
        assert_eq!(classify_path("/nasa/status/9||", None), post(Twitter, "9", None));
        // An X user named like an Instagram kind is still a tweet.
        assert_eq!(classify_path("/reel/status/20", None), post(Twitter, "20", None));
        assert_eq!(
            classify_path("/p/ABC/", Some("img_index=2")),
            post(Instagram, "ABC", Some(2))
        );
        assert_eq!(classify_path("/share/reel/xyz/", None), Some(Target::InstagramShare));
        for path in [
            "/",
            "/jack",
            "/jack/status",
            "/jack/status/abc",
            "/jack/likes/20",
            "/a/b/status/20",
            "/jack/status/123456789012345678901",
            "/someuser/",
            "/p/ABC.exe",
        ] {
            assert_eq!(classify_path(path, None), None, "{path}");
        }
    }

    #[test]
    fn parses_fxtwitter_path_modifiers() {
        use Provider::*;
        let direct = PathMods {
            direct: true,
            ..PathMods::default()
        };
        let lang = |l| PathMods {
            lang: Some(l),
            ..PathMods::default()
        };
        assert_eq!(classify_path("/jack/status/20.mp4", None), with_mods(Twitter, "20", None, direct));
        assert_eq!(classify_path("/jack/status/20.JPG", None), with_mods(Twitter, "20", None, direct));
        assert_eq!(
            classify_path("/jack/status/20.jpg:orig", None),
            with_mods(Twitter, "20", None, PathMods { image_name: Some("orig"), ..direct })
        );
        assert_eq!(classify_path("/dl/jack/status/20", None), with_mods(Twitter, "20", None, direct));
        assert_eq!(classify_path("/dir/jack/status/20", None), with_mods(Twitter, "20", None, direct));
        assert_eq!(classify_path("/jack/status/20/ja", None), with_mods(Twitter, "20", None, lang("ja")));
        assert_eq!(
            classify_path("/jack/status/20/photo/2/en", None),
            with_mods(Twitter, "20", Some(2), lang("en"))
        );
        assert_eq!(classify_path("/reel/ABC.mp4", None), with_mods(Instagram, "ABC", None, direct));
        assert_eq!(classify_path("/p/ABC/de/", None), with_mods(Instagram, "ABC", None, lang("de")));
    }

    #[test]
    fn rejects_non_posts() {
        for input in [
            "https://www.instagram.com/someuser/",
            "https://www.instagram.com/reels/",
            "https://www.instagram.com/reels/audio/123/",
            "https://www.instagram.com/p/bad%20code/",
            "https://www.instagram.com/stories/someuser/123/",
            "https://x.com/elonmusk",
            "https://x.com/elonmusk/status/notanid",
            // Paths are only valid on their own platform's host.
            "https://x.com/p/ABC/",
            "https://www.instagram.com/jack/status/20",
            "https://example.com/p/ABC/",
            "https://fix.example.com/p/ABC/",
            "ftp://www.instagram.com/p/ABC/",
            "not a url",
        ] {
            assert_eq!(rw(input), None, "{input}");
        }
    }
}
