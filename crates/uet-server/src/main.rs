mod activity;
mod config;
mod embed;
mod instagram;
mod mosaic;
mod post;
mod translate;
mod twitter;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use moka::{Expiry, future::Cache};
use serde::Deserialize;
use tracing::{info, warn};
use uet_core::{Provider, Target};

use crate::{
    activity::Snowcode,
    config::{Config, MediaMode},
    embed::{Agent, Flags, Page},
    instagram::Instagram,
    post::{FetchError, Media, Post, sized_image},
    translate::{Translated, Translator, target_code},
    twitter::Twitter,
};

const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

type PostKey = (Provider, String);

struct AppState {
    cfg: Config,
    instagram: Instagram,
    twitter: Twitter,
    /// DeepL, when `DEEPL_API_KEY` is set.
    translator: Option<Translator>,
    /// Plain client for CDN media: proxy mode and mosaic tiles (never via `UPSTREAM_PROXY`).
    media: reqwest::Client,
    posts: Cache<PostKey, Arc<Post>>,
    /// Instagram `/share/...` path → shortcode.
    shares: Cache<String, String>,
    /// `None` = nothing to translate (no text, or already in the target language).
    translations: Cache<(Provider, String, &'static str), Arc<Option<Translated>>>,
    mosaics: Cache<PostKey, Bytes>,
}

type Shared = Arc<AppState>;

/// Expires cached posts before their signed CDN URLs do.
struct PostExpiry(Duration);

impl Expiry<PostKey, Arc<Post>> for PostExpiry {
    fn expire_after_create(
        &self,
        _key: &PostKey,
        post: &Arc<Post>,
        _created_at: std::time::Instant,
    ) -> Option<Duration> {
        Some(post.url_lifetime().map_or(self.0, |l| l.min(self.0)))
    }
}

impl AppState {
    async fn post(&self, provider: Provider, id: &str) -> Result<Arc<Post>, Arc<FetchError>> {
        // Single-flight: concurrent unfurls of the same link share one upstream fetch.
        self.posts
            .try_get_with((provider, id.to_owned()), async {
                let post = match provider {
                    Provider::Instagram => self.instagram.fetch_post(id).await?,
                    Provider::Twitter => self.twitter.fetch_post(id).await?,
                };
                Ok(Arc::new(post))
            })
            .await
    }

    async fn share_shortcode(&self, path: &str) -> Result<String, Arc<FetchError>> {
        self.shares
            .try_get_with_by_ref(path, self.instagram.resolve_share(path))
            .await
    }

    /// Translation of `post` into DeepL `target`. Errors are logged and render untranslated.
    async fn translation(
        &self,
        translator: &Translator,
        post: &Post,
        target: &'static str,
    ) -> Arc<Option<Translated>> {
        let key = (post.provider, post.id.clone(), target);
        let result = self
            .translations
            .try_get_with(key, async {
                translator.translate_post(post, target).await.map(Arc::new)
            })
            .await;
        result.unwrap_or_else(|e: Arc<FetchError>| {
            warn!(provider = post.provider.slug(), id = post.id, target, error = %e, "translation failed");
            Arc::new(None)
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,uet_server=info".into()),
        )
        .init();
    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(2);
        }
    };
    let api = api_client(&cfg).expect("api client");
    let media = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // Pass bytes through untouched so forwarded Content-Length stays valid.
        .no_gzip()
        .no_brotli()
        .build()
        .expect("media client");
    let state = Arc::new(AppState {
        instagram: Instagram::new(api.clone(), cfg.doc_id.clone()),
        translator: cfg
            .deepl_api_key
            .clone()
            .map(|key| Translator::new(api.clone(), key)),
        twitter: Twitter::new(api),
        media,
        posts: Cache::builder()
            .max_capacity(cfg.cache_capacity)
            .expire_after(PostExpiry(cfg.cache_ttl))
            .build(),
        shares: Cache::builder()
            .max_capacity(cfg.cache_capacity)
            .time_to_live(Duration::from_secs(24 * 3600))
            .build(),
        translations: Cache::builder()
            .max_capacity(cfg.cache_capacity)
            .time_to_live(Duration::from_secs(24 * 3600))
            .build(),
        mosaics: Cache::builder()
            // Weighted by JPEG size: at most 256 MiB of mosaics.
            .weigher(|_, jpeg: &Bytes| u32::try_from(jpeg.len()).unwrap_or(u32::MAX))
            .max_capacity(256 << 20)
            .time_to_live(cfg.cache_ttl)
            .build(),
        cfg,
    });

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/owoembed", get(owoembed))
        .route("/api/v1/statuses/{snowcode}", get(activity_status))
        .route("/mosaic/{provider}/{file}", get(mosaic))
        .route("/media/{provider}/{id}/{n}/video.mp4", get(media_video))
        .route("/media/{provider}/{id}/{n}/image.jpg", get(media_image))
        .fallback(get(post_page))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(state.cfg.bind)
        .await
        .expect("bind");
    info!(
        bind = %state.cfg.bind,
        public_url = %state.cfg.public_url,
        translation = state.translator.is_some(),
        "listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server");
}

/// Client for Instagram/X metadata requests. Never follows redirects: on instagram.com a
/// redirect means login wall, and share links are resolved by reading `Location`.
fn api_client(cfg: &Config) -> reqwest::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    if let Some(proxy) = &cfg.upstream_proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    builder.build()
}

/// Every instagram.com- or x.com-shaped post path, on the fix host or a modifier subdomain.
/// Direct links go to the media file, crawlers get an embed page, people get the original site.
async fn post_page(State(s): State<Shared>, uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path();
    let query = uri.query();
    let Some(target) = uet_core::classify_path(path, query) else {
        return (StatusCode::NOT_FOUND, "not an Instagram or X post link\n").into_response();
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or(uri.host())
        .unwrap_or_default();
    let label = embed::host_label(host, &s.cfg.public_host);
    // Activity links must point back at the modifier subdomain the request came in on.
    let origin = match label {
        Some(_) => {
            let scheme = s.cfg.public_url.split_once("://").map_or("https", |(scheme, _)| scheme);
            format!("{scheme}://{host}")
        }
        None => s.cfg.public_url.clone(),
    };

    let (provider, id, handle, index, mods) = match target {
        Target::Post { provider, id, handle, index, mods } => {
            (provider, id.to_owned(), handle, index, mods)
        }
        Target::InstagramShare => {
            let fallback = format!("{}{path}", embed::site_origin(Provider::Instagram));
            match s.share_shortcode(path).await {
                Ok(code) => (Provider::Instagram, code, None, None, Default::default()),
                Err(e) => {
                    warn!(path, error = %e, "share link resolution failed");
                    return Redirect::to(&fallback).into_response();
                }
            }
        }
    };
    let flags = Flags::new(label.unwrap_or_default(), mods.direct);
    let original_url = match provider {
        Provider::Twitter => format!("https://x.com/{}/status/{id}", handle.unwrap_or("i")),
        Provider::Instagram if mods == Default::default() && !path.starts_with("/share/") => {
            let origin = embed::site_origin(provider);
            match query {
                Some(q) => format!("{origin}{path}?{q}"),
                None => format!("{origin}{path}"),
            }
        }
        Provider::Instagram => format!("{}/p/{id}/", embed::site_origin(provider)),
    };
    let bot = embed::is_bot(user_agent);
    if !bot && !flags.direct {
        return Redirect::to(&original_url).into_response();
    }

    let post = match s.post(provider, &id).await {
        Ok(post) => post,
        Err(e) => {
            // Fall back to the site's own preview.
            warn!(provider = provider.slug(), id, error = %e, "fetch failed");
            return Redirect::to(&original_url).into_response();
        }
    };
    let index = index
        .map(|i| i.get() as usize - 1)
        .filter(|&i| i < post.media.len());

    if flags.direct
        && let Some(m) = post.media.get(index.unwrap_or(0))
    {
        let name = mods.image_name.map(str::to_owned).or_else(|| query_param(query, "name"));
        let url = match (&m.video, name) {
            (Some(video), _) => video.clone(),
            (None, Some(name)) => sized_image(provider, &m.image, &name),
            (None, None) => m.image.clone(),
        };
        return deliver(&s, &url, &headers).await;
    }
    if !bot {
        return Redirect::to(&original_url).into_response();
    }

    let lang = s.translator.as_ref().and(mods.lang).and_then(target_code);
    let translated;
    let mut page = Page {
        post: &post,
        flags,
        agent: Agent::new(user_agent),
        index,
        translation: None,
        lang,
        base: &s.cfg.public_url,
        origin: &origin,
    };
    // Discord's activity embed translates on its own request (snowcode `l`).
    if let (Some(translator), Some(target)) = (&s.translator, lang)
        && !page.uses_activity()
    {
        translated = s.translation(translator, &post, target).await;
        page.translation = translated.as_ref().as_ref();
    }
    (
        [
            (header::CACHE_CONTROL, "private, max-age=0"),
            (header::VARY, "User-Agent"),
        ],
        Html(embed::render_html(&page)),
    )
        .into_response()
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query?.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

/// Mastodon API status Discord fetches for the activity link in the embed page.
async fn activity_status(State(s): State<Shared>, Path(snowcode): Path<String>) -> Response {
    let Some(code) = Snowcode::decode(&snowcode) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(provider) = code.provider().filter(|p| p.is_valid_id(&code.i)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let post = match s.post(provider, &code.i).await {
        Ok(post) => post,
        Err(e) => {
            warn!(provider = provider.slug(), id = code.i, error = %e, "activity fetch failed");
            return error_status(&e).into_response();
        }
    };
    let translated = match (&s.translator, code.l.as_deref().and_then(target_code)) {
        (Some(translator), Some(target)) => Some(s.translation(translator, &post, target).await),
        _ => None,
    };
    let translation = translated.as_deref().and_then(Option::as_ref);
    Json(activity::status(&post, &code, translation, &s.cfg.public_url)).into_response()
}

#[derive(Deserialize)]
struct OwoEmbedQuery {
    text: Option<String>,
    status: String,
    author: String,
    provider: Option<String>,
    realm: Option<String>,
}

/// oEmbed Discord reads for the author/provider lines (FxEmbed's `/owoembed`).
async fn owoembed(Query(q): Query<OwoEmbedQuery>) -> Response {
    let provider = match q.realm.as_deref() {
        None => Provider::Twitter,
        Some(realm) => match activity::provider_for_realm(realm) {
            Some(provider) => provider,
            None => return StatusCode::BAD_REQUEST.into_response(),
        },
    };
    Json(embed::owoembed(provider, q.text, &q.status, &q.author, q.provider)).into_response()
}

/// Self-hosted FxEmbed mosaic: 2-4 photos tiled into one JPEG.
async fn mosaic(
    State(s): State<Shared>,
    Path((provider, file)): Path<(String, String)>,
) -> Response {
    let Some((provider, id)) = file.strip_suffix(".jpg").and_then(|id| {
        Provider::from_slug(&provider)
            .filter(|p| p.is_valid_id(id))
            .map(|p| (p, id))
    }) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let post = match s.post(provider, id).await {
        Ok(post) => post,
        Err(e) => return error_status(&e).into_response(),
    };
    let Some(tiles) = post.mosaic_tiles() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let jpeg = s
        .mosaics
        .try_get_with((provider, id.to_owned()), build_mosaic(s.media.clone(), tiles))
        .await;
    match jpeg {
        Ok(jpeg) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            jpeg,
        )
            .into_response(),
        Err(e) => {
            warn!(provider = provider.slug(), id, error = %e, "mosaic failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

async fn build_mosaic(
    client: reqwest::Client,
    tiles: Vec<(String, (u32, u32))>,
) -> Result<Bytes, String> {
    let downloads: Vec<_> = tiles
        .iter()
        .map(|(url, _)| {
            let req = client.get(url).send();
            tokio::spawn(async move {
                post::check_status(req.await?)?.bytes().await.map_err(FetchError::from)
            })
        })
        .collect();
    let mut files = Vec::with_capacity(downloads.len());
    for download in downloads {
        files.push(download.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?);
    }
    tokio::task::spawn_blocking(move || {
        let images = files
            .iter()
            .zip(&tiles)
            .map(|(file, (_, (w, h)))| {
                let image = image::load_from_memory(file)?.into_rgb8();
                // Match the size the page advertised in `og:image:width/height`.
                Ok(if image.dimensions() == (*w, *h) {
                    image
                } else {
                    image::imageops::resize(&image, *w, *h, image::imageops::FilterType::Triangle)
                })
            })
            .collect::<Result<Vec<_>, image::ImageError>>()
            .map_err(|e| e.to_string())?;
        let canvas = mosaic::render(images).ok_or("not 2-4 images")?;
        mosaic::encode_jpeg(&canvas).map(Bytes::from).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn media_video(
    State(s): State<Shared>,
    Path((provider, id, n)): Path<(String, String, usize)>,
    headers: HeaderMap,
) -> Response {
    serve_media(&s, &provider, &id, n, &headers, |m| m.video.as_deref()).await
}

async fn media_image(
    State(s): State<Shared>,
    Path((provider, id, n)): Path<(String, String, usize)>,
    headers: HeaderMap,
) -> Response {
    serve_media(&s, &provider, &id, n, &headers, |m| Some(&m.image)).await
}

/// Resolves a fresh CDN URL for media item `n` (1-based); redirects or streams it.
async fn serve_media(
    s: &AppState,
    provider: &str,
    id: &str,
    n: usize,
    headers: &HeaderMap,
    pick: impl Fn(&Media) -> Option<&str>,
) -> Response {
    let Some(provider) = Provider::from_slug(provider).filter(|p| p.is_valid_id(id) && n > 0)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let post = match s.post(provider, id).await {
        Ok(post) => post,
        Err(e) => {
            warn!(provider = provider.slug(), id, error = %e, "media lookup failed");
            return error_status(&e).into_response();
        }
    };
    let Some(url) = post.media.get(n - 1).and_then(&pick) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    deliver(s, url, headers).await
}

/// Hands a CDN URL to the client per `MEDIA_MODE`: 302 or streamed.
async fn deliver(s: &AppState, url: &str, headers: &HeaderMap) -> Response {
    match s.cfg.media_mode {
        MediaMode::Redirect => (
            StatusCode::FOUND,
            [
                (header::LOCATION, url),
                (header::CACHE_CONTROL, "public, max-age=300"),
            ],
        )
            .into_response(),
        MediaMode::Proxy => proxy(&s.media, url, headers).await,
    }
}

async fn proxy(client: &reqwest::Client, url: &str, headers: &HeaderMap) -> Response {
    let mut req = client.get(url);
    if let Some(range) = headers.get(header::RANGE) {
        req = req.header(header::RANGE, range);
    }
    let upstream = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "media proxy failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut resp = Response::builder().status(upstream.status());
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::LAST_MODIFIED,
        header::ETAG,
    ] {
        if let Some(v) = upstream.headers().get(&name) {
            resp = resp.header(name, v);
        }
    }
    resp.header(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"))
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn error_status(e: &FetchError) -> StatusCode {
    match e {
        FetchError::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_GATEWAY,
    }
}
