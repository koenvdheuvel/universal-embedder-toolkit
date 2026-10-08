# universal-embedder-toolkit

Fixes Discord (and Telegram, Slack, Mastodon, …) previews for Instagram and X/Twitter posts so
videos play inline. Self-hosted, Rust, no login, no third-party fixer service. One host serves both.

```
https://www.instagram.com/reels/DeMPFloOrjv/        →   https://fix.example.com/reels/DeMPFloOrjv/
https://x.com/elonmusk/status/1585341984679469056    →   https://fix.example.com/elonmusk/status/1585341984679469056
```

| Part | What it does |
| --- | --- |
| `crates/uet-server` | Embed server. Crawlers get OpenGraph/Twitter-card HTML pointing at the MP4; browsers get a redirect to the original post. |
| `crates/uet-clip` | Desktop clipboard watcher (macOS/Linux/Windows): copy an Instagram/X link, it becomes the fix link. |
| `crates/uet-core` | Shared URL recognition/rewrite rules. |
| `android/` | Android app: share-sheet target, text-selection action and Quick Settings tile. |

## How the server works

1. Paths mirror the original sites, and they don't overlap:
   - Instagram: `/p/{code}`, `/reel/{code}`, `/reels/{code}`, `/tv/{code}`, `/{user}/p/{code}`,
     `/share/...` (resolved via Instagram's redirect); `?img_index=N` picks a carousel item.
   - X: `/{user}/status/{id}` (also `statuses`/`article`), `/status/{id}`, `/i/web/status/{id}`;
     `/photo/N` or `/video/N` picks a media item.
2. User-Agent is a link-preview bot (FxTwitter's bot list: `Discordbot`, Discord's `Firefox/38` fetcher,
   `TelegramBot`, `WhatsApp`, `Slackbot`, …)? → fetch the post and return FxTwitter-style embed HTML.
   Anyone else → `303` to the original post, so clicking the link in Discord opens Instagram/X.
3. Data sources, all unauthenticated:
   - Instagram: web GraphQL endpoint (one ~20 KB JSON request; any self-generated CSRF token works),
     falling back to the public embed page (`/p/{code}/embed/captioned/`, `contextJSON`).
   - X: `cdn.syndication.twimg.com/tweet-result` (the endpoint behind embedded tweets). Videos use the
     highest-bitrate MP4 whose estimated size is ≤ 100 MB (Discord stops inlining very large linked videos).
4. `og:video` points at `/media/{ig|x}/{id}/{n}/video.mp4` on this server, not at the CDN directly.
   That route `302`s to the current CDN URL, so old Discord messages keep playing after Instagram's
   signed URL (`oe=` expiry, a few days) runs out. `MEDIA_MODE=proxy` streams the bytes instead (Range supported).
5. Posts are cached in memory (moka, single-flight) until 10 min before the earliest signed CDN URL expires,
   capped at `CACHE_TTL_SECS`. Discord fetching the page, oEmbed and media at once costs one upstream request.

Failure mode: if Instagram/X refuses (or the tweet is deleted, protected or age-restricted), crawlers are
redirected to the original URL, so you get the site's normal preview instead of nothing.

### FxTwitter-compatible links

The embeds follow [FxEmbed](https://github.com/FxEmbed/FxEmbed)'s rendering rules, and its link tricks work
on the fix host:

| Link | Effect |
| --- | --- |
| `d.fix.example.com/...`, `dl.…`, `/{id}.mp4`, `/{id}.jpg`, `/dl/{user}/status/{id}` | Direct media: `302` to the file itself, for everyone (browsers too). With `/photo/N` picks that item. Photos take a size via `.jpg:orig` or `?name=orig` (X only). |
| `t.…` | Text only: no images or video. |
| `g.…` | Gallery: media and author only, no text or engagement. |
| `m.…` | Always one mosaic image, even on Discord. |
| `o.…` | Old-style embed: no Discord activity embed. |
| `i.…` | Telegram Instant View for every post. |
| `.../status/{id}/ja`, `/p/{code}/de` | Translate the post (and its quote) with DeepL; needs `DEEPL_API_KEY`, ignored otherwise. |

What the page contains depends on the client, as on FxTwitter:

- **Discord** gets an *activity* link (`/users/{user}/statuses/{snowcode}` → `/api/v1/statuses/{snowcode}`,
  a Mastodon v1 status). Discord renders that as a rich post: linked mentions/hashtags/URLs, quote block,
  reply line, 💬/❤️ counts, and every photo or video of the post. The snowcode is FxEmbed's encoding of
  the options (`t`, `m`, `n`, `l`) as digits.
- **Other bots** (Slack, WhatsApp, Mastodon, …) get OpenGraph/Twitter-card tags: video player tags,
  the photo, or a **mosaic** of 2-4 photos at `/mosaic/{ig|x}/{id}.jpg`. The mosaic is rendered on this
  server by a port of [FxEmbed/mosaic](https://github.com/FxEmbed/mosaic) (MIT, `crates/uet-server/src/mosaic/`)
  and cached in memory (256 MiB cap). Clients that show several `og:image` tags (Discord with `o.`,
  Matrix) get one tag per photo instead.
- **Telegram** gets an Instant View article for photos, quotes, translations and multi-video posts.
- Every non-gallery page links `/owoembed`, the oEmbed document Discord shows as the author line
  (engagement, the post text for videos, "Photo 2 / 4" counters, "GIF - X").

Not ported, because syndication doesn't expose it: view/repost counts, polls, threads, articles,
NSFW/protected tweets, and profile stats in the activity account (they show as 0).

### Translation (DeepL)

Create a DeepL API key (API Free works: 500,000 characters/month; keys ending in `:fx` use the free
endpoint automatically) and set `DEEPL_API_KEY`. Append a language code to any post link: `/en`, `/ja`,
`/pt-br`, `/zh`, … Text and quote text go out in one request; results are cached for 24 h per post and
language. Posts already in the target language, and failed requests, render untranslated.

## Hosting: your NL server, not Cloudflare Workers

CPU is not the problem: a request is one JSON parse (~1 ms), well under even the free Workers 10 ms CPU
limit, and waiting on upstream doesn't count as CPU. The problem is **egress IP reputation**. Instagram
login-walls and rate-limits shared datacenter/proxy ranges, and Workers egress from Cloudflare's shared
pool that every scraper uses — the same reason ddinstagram-style services keep dying. You can't pin or
change a Worker's egress IP. On your own server you have one stable IP doing a low, cached request volume,
and if it ever gets flagged you can set `UPSTREAM_PROXY` to a residential proxy for API calls only (media
never goes through it). Putting Cloudflare's proxy (orange cloud) *in front* of the server is fine;
responses send `Vary: User-Agent` / `private` so bot and human responses don't get mixed.

### Deploy (Docker + Caddy for TLS)

Point DNS at the server for `fix.example.com` and the modifier subdomains (`d`, `dl`, `t`, `g`, `m`, `o`,
`i`; a `*.fix.example.com` wildcard record covers them), then on the server:

```sh
git clone <this repo> && cd universal-embedder-toolkit
FIX_DOMAIN=fix.example.com DEEPL_API_KEY=... docker compose -f deploy/compose.yaml up -d --build
curl -A Discordbot https://fix.example.com/reels/DeMPFloOrjv/                    # og:video tags
curl -A Discordbot https://fix.example.com/elonmusk/status/1585341984679469056   # activity link
curl -I https://d.fix.example.com/elonmusk/status/1585341984679469056            # 302 to the MP4
```

Caddy gets a certificate per subdomain (`deploy/Caddyfile`).

Without Docker: `cargo build --release -p uet-server`, run `target/release/uet-server` behind any TLS proxy.

### Configuration (environment)

| Variable | Default | Purpose |
| --- | --- | --- |
| `PUBLIC_URL` | `http://localhost:8080` | Public base URL used in generated tags; its host is where modifier subdomains (`d.`, `t.`, …) are recognised. |
| `DEEPL_API_KEY` | – | Enables `/{lang}` translation. |
| `BIND` | `0.0.0.0:8080` | Listen address. |
| `MEDIA_MODE` | `redirect` | `redirect` to CDN or `proxy` (stream through server). |
| `UPSTREAM_PROXY` | – | `http://` / `socks5h://` proxy for Instagram/X API requests only. |
| `IG_DOC_ID` | `25018359077785073` | Instagram GraphQL query id. Instagram rotates these; when it goes stale the embed-page fallback still works. |
| `CACHE_TTL_SECS` / `CACHE_CAPACITY` | `3600` / `10000` | Post cache bounds. |
| `RUST_LOG` | `info` | e.g. `uet_server=debug` logs which fetch strategy was used. |

## Desktop clipboard rewriting (`uet-clip`)

Rewrites the clipboard only when it holds exactly one Instagram post/reel/share link or X status link.
Tracking params (`igsh`, `?s=20&t=…`) are dropped; Instagram's `img_index` and X's `/photo/N` are kept.
On macOS it polls NSPasteboard's change counter every 300 ms, so it reads nothing while idle.

```sh
cargo build --release -p uet-clip
cp target/release/uet-clip /usr/local/bin/
uet-clip https://fix.example.com          # foreground test

# start at login (edit the URL in the plist first)
cp deploy/macos/dev.uet.clip.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.uet.clip.plist
```

## Android

Android 10+ only lets the focused app (or the keyboard) read the clipboard, so silent background
rewriting like `uet-clip` is impossible without root. The app gives three one-tap paths instead:

- **Share sheet → "Fix embed"**: in Instagram or X tap Share → Fix embed. The fixed link is put on the clipboard.
- **Text selection → "Fix embed"**: paste the link in Discord's message box, select it, tap Fix embed.
  The link is replaced in place.
- **Quick Settings tile "Fix link"**: after "Copy link" in Instagram or X, pull down the shade and tap the
  tile. It rewrites the clipboard.

Plain framework activities, no AndroidX; only the Kotlin stdlib at runtime. Needs JDK 17 + Android SDK (platform 35).

```sh
cd android
./gradlew assembleDebug -PfixBaseUrl=https://fix.example.com
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

You can also change the fix URL in the app: open it from the launcher, edit the URL, tap Save.

## CI and releases

### Server image (GHCR)

Every push to `main` and every `v*` tag runs the `image` workflow: `cargo test --workspace`, then native
`linux/amd64` + `linux/arm64` builds of the root `Dockerfile`, merged into one multi-arch image.

```sh
docker pull ghcr.io/koenvdheuvel/universal-embedder-toolkit:latest   # main
docker pull ghcr.io/koenvdheuvel/universal-embedder-toolkit:0.1.0    # a release tag
docker pull ghcr.io/koenvdheuvel/universal-embedder-toolkit:sha-abc1234
```

Tags: `latest` (main), `sha-<short>`, and `X.Y.Z` / `X.Y` for `vX.Y.Z` git tags. The package is public, no login needed.

### Android APK and macOS `uet-clip`

Pushing a `v*` tag runs the `release` workflow and attaches to the
[GitHub release](https://github.com/koenvdheuvel/universal-embedder-toolkit/releases):

- `uet-embedfix-vX.Y.Z.apk`: signed release APK. The default fix URL is the repository variable `FIX_BASE_URL`
  (falls back to `https://fix.example.com`); change it in the app after installing.
- `uet-clip-vX.Y.Z-macos-universal.zip`: universal (arm64 + x86_64) `uet-clip`, the launchd plist and an install note.
  The binary is ad-hoc signed, not notarized, so clear the quarantine flag after downloading:
  `xattr -d com.apple.quarantine uet-clip` (then follow `INSTALL.txt` / the [clipboard section](#desktop-clipboard-rewriting-uet-clip)).

Release signing reads `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and
`ANDROID_KEY_PASSWORD` from the environment (CI sets them from the repo secrets
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`). Without them,
`assembleRelease` produces an unsigned APK and debug builds are unaffected. Keep the keystore: updates must be signed
with the same key as the installed app.
