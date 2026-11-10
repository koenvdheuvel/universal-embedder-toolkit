//! Fetches public post data from Instagram without logging in.
//!
//! Strategy: the logged-out web GraphQL query `PolarisPostRootQuery` (one small JSON
//! request), falling back to the public embed page (`/p/{code}/embed/captioned/`), which used
//! to carry the same data in `contextJSON`. Since November 2026 that page is script-only for
//! most posts, so the fallback rarely works.
//!
//! The query matches what InstaFix-Revived and OGInstagram send since October 2026: doc id
//! `27128499623469141`, a random `lsd` token, and no CSRF cookie or app id header.

use reqwest::{StatusCode, header};
use serde::Deserialize;
use uet_core::Provider;
use url::Url;

use crate::post::{FetchError, Media, Post, check_status, iso8601};

const GRAPHQL_URL: &str = "https://www.instagram.com/graphql/query/";
/// The logged-out query is sent with this bare user agent, not a full browser one.
const GRAPHQL_UA: &str = "Mozilla/5.0";

pub struct Instagram {
    /// Must not follow redirects: on instagram.com a redirect means login wall.
    api: reqwest::Client,
    doc_id: String,
}

impl Instagram {
    pub fn new(api: reqwest::Client, doc_id: String) -> Self {
        Self { api, doc_id }
    }

    pub async fn fetch_post(&self, shortcode: &str) -> Result<Post, FetchError> {
        let mut post = match self.fetch_graphql(shortcode).await {
            Ok(post) => post,
            // Only an HTTP 404 means the post is gone. An empty GraphQL answer also happens
            // when the query id is stale or the request is refused, so the embed page still runs.
            Err(FetchError::NotFound) => return Err(FetchError::NotFound),
            Err(gql) => match self.fetch_embed(shortcode).await {
                Ok(post) => {
                    tracing::warn!(shortcode, error = %gql, "graphql failed, embed page worked");
                    post
                }
                Err(embed) => {
                    tracing::warn!(shortcode, graphql = %gql, embed = %embed, "every instagram source failed");
                    return Err(embed);
                }
            },
        };
        post.id = shortcode.to_owned();
        Ok(post)
    }

    async fn fetch_graphql(&self, shortcode: &str) -> Result<Post, FetchError> {
        // Any random token works as `lsd`, as long as the header and the form field agree.
        let lsd: String = std::iter::repeat_with(fastrand::alphanumeric).take(24).collect();
        let variables = serde_json::json!({
            "shortcode": shortcode,
            "__relay_internal__pv__PolarisAIGMMediaWebLabelEnabledrelayprovider": false,
        })
        .to_string();
        let resp = self
            .api
            .post(GRAPHQL_URL)
            .header(header::USER_AGENT, GRAPHQL_UA)
            .header(header::ACCEPT, "*/*")
            .header("X-FB-Friendly-Name", "PolarisPostRootQuery")
            .header("X-FB-LSD", &lsd)
            .form(&[
                ("variables", variables.as_str()),
                ("doc_id", &self.doc_id),
                ("server_timestamps", "true"),
                ("lsd", &lsd),
            ])
            .send()
            .await?;
        let resp = check_status(resp)?;
        let body: GqlResponse = resp.json().await?;
        body.check()?;
        let item = body
            .data
            .and_then(|d| d.info)
            .and_then(|i| i.items.into_iter().next())
            .ok_or_else(|| FetchError::Parse("graphql returned no media (stale IG_DOC_ID, refused, or post gone)".into()))?;
        item.into_post()
    }

    async fn fetch_embed(&self, shortcode: &str) -> Result<Post, FetchError> {
        let resp = self
            .api
            .get(format!(
                "https://www.instagram.com/p/{shortcode}/embed/captioned/"
            ))
            .header(header::ACCEPT, "text/html,application/xhtml+xml")
            .header(header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .send()
            .await?;
        let html = check_status(resp)?.text().await?;
        parse_embed(&html)
    }

    /// Resolves a `/share/...` path to a shortcode by reading Instagram's redirect.
    pub async fn resolve_share(&self, path: &str) -> Result<String, FetchError> {
        let resp = self
            .api
            .get(format!("https://www.instagram.com{path}"))
            .header(header::ACCEPT, "text/html")
            .send()
            .await?;
        if !resp.status().is_redirection() {
            return Err(match resp.status() {
                StatusCode::NOT_FOUND => FetchError::NotFound,
                s => FetchError::Blocked(s),
            });
        }
        let location = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| FetchError::Parse("share redirect without Location".into()))?;
        let target = Url::parse("https://www.instagram.com")
            .and_then(|base| base.join(location))
            .map_err(|e| FetchError::Parse(e.to_string()))?;
        match uet_core::classify_instagram(target.path(), None) {
            Some(uet_core::Target::Post { id, .. }) => Ok(id.to_owned()),
            // Login wall or another share hop.
            _ => Err(FetchError::Blocked(StatusCode::FOUND)),
        }
    }
}

// ---- GraphQL (`xdt_api__v1__media__shortcode__web_info`, private-API media shape) ----

#[derive(Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
    /// GraphQL reports rate limits in a 200 response.
    #[serde(default)]
    errors: Vec<GqlError>,
    /// `"fail"` with a `message` when Instagram refuses the request.
    status: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct GqlError {
    code: Option<i64>,
    message: Option<String>,
}

impl GqlResponse {
    fn check(&self) -> Result<(), FetchError> {
        if self.status.as_deref() == Some("fail") {
            let msg = self.message.as_deref().unwrap_or("no message");
            return Err(FetchError::Parse(format!("instagram refused the query: {msg}")));
        }
        match self.errors.first() {
            Some(e) if e.message.as_deref().is_some_and(|m| m.contains("Please wait")) => {
                Err(FetchError::Blocked(StatusCode::TOO_MANY_REQUESTS))
            }
            // Only an error without data is a failure; partial errors still carry the post.
            Some(e) if self.data.is_none() => Err(FetchError::Parse(format!(
                "graphql error {}: {}",
                e.code.unwrap_or_default(),
                e.message.as_deref().unwrap_or("no message")
            ))),
            _ => Ok(()),
        }
    }
}

#[derive(Deserialize)]
struct GqlData {
    #[serde(rename = "xdt_api__v1__media__shortcode__web_info")]
    info: Option<WebInfo>,
}

#[derive(Deserialize)]
struct WebInfo {
    items: Vec<ApiMedia>,
}

#[derive(Deserialize)]
struct ApiMedia {
    video_versions: Option<Vec<ApiVariant>>,
    image_versions2: Option<ApiImages>,
    carousel_media: Option<Vec<ApiMedia>>,
    user: Option<ApiUser>,
    caption: Option<ApiCaption>,
    like_count: Option<u64>,
    comment_count: Option<u64>,
    taken_at: Option<u64>,
    accessibility_caption: Option<String>,
}

#[derive(Deserialize)]
struct ApiVariant {
    url: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct ApiImages {
    candidates: Vec<ApiVariant>,
}

#[derive(Deserialize)]
struct ApiUser {
    username: String,
    full_name: Option<String>,
    profile_pic_url: Option<String>,
}

#[derive(Deserialize)]
struct ApiCaption {
    text: String,
}

impl ApiMedia {
    fn to_media(&self) -> Option<Media> {
        // Candidates and versions are sorted best-first.
        let image = self.image_versions2.as_ref()?.candidates.first()?;
        let video = self.video_versions.as_ref().and_then(|v| v.first());
        let (width, height) = video.map_or((image.width, image.height), |v| (v.width, v.height));
        Some(Media {
            video: video.map(|v| v.url.clone()),
            image: image.url.clone(),
            width,
            height,
            alt: self.accessibility_caption.clone(),
            gif: false,
        })
    }

    fn into_post(self) -> Result<Post, FetchError> {
        let media: Vec<Media> = match &self.carousel_media {
            Some(children) => children.iter().filter_map(Self::to_media).collect(),
            None => self.to_media().into_iter().collect(),
        };
        if media.is_empty() {
            return Err(FetchError::Parse("post has no media".into()));
        }
        let user = self.user.ok_or_else(|| FetchError::Parse("post has no user".into()))?;
        Ok(Post {
            provider: Provider::Instagram,
            id: String::new(),
            handle: user.username,
            display_name: user.full_name.filter(|n| !n.is_empty()),
            text: self.caption.map(|c| c.text).filter(|t| !t.is_empty()),
            lang: None,
            created_at: self.taken_at.map(iso8601),
            avatar: user.profile_pic_url,
            reply_to: None,
            likes: self.like_count,
            comments: self.comment_count,
            media,
            quote: None,
        })
    }
}

// ---- Embed page (`contextJSON` → `gql_data.shortcode_media`, legacy GraphQL shape) ----

fn parse_embed(html: &str) -> Result<Post, FetchError> {
    const KEY: &str = "\"contextJSON\":";
    let start = html
        .find(KEY)
        .ok_or_else(|| FetchError::Parse("embed page without contextJSON (post data not in the HTML)".into()))?
        + KEY.len();
    // contextJSON is a JSON string containing JSON; read exactly one value.
    let mut de = serde_json::Deserializer::from_str(&html[start..]);
    let context = Option::<String>::deserialize(&mut de)?.ok_or(FetchError::NotFound)?;
    let context: EmbedContext = serde_json::from_str(&context)?;
    context
        .gql_data
        .and_then(|d| d.shortcode_media)
        .ok_or(FetchError::NotFound)?
        .into_post()
}

#[derive(Deserialize)]
struct EmbedContext {
    gql_data: Option<EmbedGql>,
}

#[derive(Deserialize)]
struct EmbedGql {
    shortcode_media: Option<GraphMedia>,
}

#[derive(Deserialize)]
struct GraphMedia {
    video_url: Option<String>,
    display_url: String,
    dimensions: Option<Dimensions>,
    accessibility_caption: Option<String>,
    taken_at_timestamp: Option<u64>,
    owner: Option<ApiUser>,
    edge_media_to_caption: Option<Edges<CaptionNode>>,
    edge_sidecar_to_children: Option<Edges<GraphMedia>>,
    edge_liked_by: Option<Count>,
    edge_media_preview_like: Option<Count>,
    edge_media_to_comment: Option<Count>,
}

#[derive(Deserialize)]
struct Dimensions {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Edges<T> {
    edges: Vec<Node<T>>,
}

#[derive(Deserialize)]
struct Node<T> {
    node: T,
}

#[derive(Deserialize)]
struct CaptionNode {
    text: String,
}

#[derive(Deserialize)]
struct Count {
    count: u64,
}

impl GraphMedia {
    fn to_media(&self) -> Media {
        let (width, height) = self.dimensions.as_ref().map_or((0, 0), |d| (d.width, d.height));
        Media {
            video: self.video_url.clone(),
            image: self.display_url.clone(),
            width,
            height,
            alt: self.accessibility_caption.clone(),
            gif: false,
        }
    }

    fn into_post(self) -> Result<Post, FetchError> {
        let media = match &self.edge_sidecar_to_children {
            Some(children) if !children.edges.is_empty() => {
                children.edges.iter().map(|n| n.node.to_media()).collect()
            }
            _ => vec![self.to_media()],
        };
        let owner = self.owner.ok_or_else(|| FetchError::Parse("post has no owner".into()))?;
        Ok(Post {
            provider: Provider::Instagram,
            id: String::new(),
            handle: owner.username,
            display_name: owner.full_name.filter(|n| !n.is_empty()),
            text: self
                .edge_media_to_caption
                .and_then(|e| e.edges.into_iter().next())
                .map(|n| n.node.text)
                .filter(|t| !t.is_empty()),
            lang: None,
            created_at: self.taken_at_timestamp.map(iso8601),
            avatar: owner.profile_pic_url,
            reply_to: None,
            likes: self.edge_liked_by.or(self.edge_media_preview_like).map(|c| c.count),
            comments: self.edge_media_to_comment.map(|c| c.count),
            media,
            quote: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_page_without_data_is_not_a_missing_post() {
        // Instagram's embed page became a script-only page in November 2026.
        let html = r#"<!DOCTYPE html><html><head><title>Instagram</title></head><body><script type="application/json">{"__bbox":{}}</script></body></html>"#;
        assert!(matches!(parse_embed(html), Err(FetchError::Parse(_))));
    }

    #[test]
    fn graphql_errors_are_reported() {
        let parse = |json: &str| serde_json::from_str::<GqlResponse>(json).unwrap().check();
        assert!(matches!(
            parse(r#"{"errors":[{"code":1675004,"message":"Please wait a few minutes before you try again."}]}"#),
            Err(FetchError::Blocked(StatusCode::TOO_MANY_REQUESTS))
        ));
        assert!(matches!(parse(r#"{"status":"fail","message":"useragent mismatch"}"#), Err(FetchError::Parse(_))));
        assert!(matches!(parse(r#"{"errors":[{"code":1,"message":"x"}]}"#), Err(FetchError::Parse(_))));
        assert!(parse(r#"{"data":{"xdt_api__v1__media__shortcode__web_info":null},"errors":[{"message":"partial"}]}"#).is_ok());
    }

    #[test]
    fn parses_embed_context_json() {
        let context = r#"{"gql_data":{"shortcode_media":{"display_url":"https://cdn.test/a.jpg","dimensions":{"width":1080,"height":1350},"owner":{"username":"ah"},"edge_media_to_caption":{"edges":[{"node":{"text":"hi"}}]}}}}"#;
        let html = format!(r#"x "contextJSON":{} y"#, serde_json::to_string(context).unwrap());
        let post = parse_embed(&html).unwrap();
        assert_eq!(post.handle, "ah");
        assert_eq!(post.text.as_deref(), Some("hi"));
        assert_eq!(post.media.len(), 1);
    }
}
