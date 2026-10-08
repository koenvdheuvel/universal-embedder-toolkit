//! DeepL translation backend for the `/{lang}` link suffix.

use serde::Deserialize;

use crate::post::{FetchError, Post, check_status};

const FREE_ENDPOINT: &str = "https://api-free.deepl.com/v2/translate";
const PRO_ENDPOINT: &str = "https://api.deepl.com/v2/translate";

/// DeepL client bound to an API key and the matching plan endpoint.
pub struct Translator {
    client: reqwest::Client,
    endpoint: &'static str,
    key: String,
}

/// Result of one translation request.
pub struct Translation {
    /// Detected source language, lowercase ISO 639-1 (DeepL `detected_source_language`, e.g. "ja").
    pub source: String,
    /// Same order/length as the input texts.
    pub texts: Vec<String>,
}

#[derive(Deserialize)]
struct Response {
    translations: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    detected_source_language: String,
    text: String,
}

impl Translator {
    /// Keys ending in ":fx" are DeepL Free keys -> api-free.deepl.com, others -> api.deepl.com.
    pub fn new(client: reqwest::Client, key: String) -> Self {
        let endpoint = if key.ends_with(":fx") { FREE_ENDPOINT } else { PRO_ENDPOINT };
        Self { client, endpoint, key }
    }

    /// One POST /v2/translate request with all texts; `target` is a DeepL code from [`target_code`].
    pub async fn translate(&self, texts: &[&str], target: &'static str) -> Result<Translation, FetchError> {
        let resp = self
            .client
            .post(self.endpoint)
            .header("Authorization", format!("DeepL-Auth-Key {}", self.key))
            .json(&serde_json::json!({ "text": texts, "target_lang": target }))
            .send()
            .await?;
        let body: Response = check_status(resp)?.json().await?;
        if body.translations.len() != texts.len() {
            return Err(FetchError::Parse("DeepL returned a mismatched translation count".into()));
        }
        let source = body
            .translations
            .first()
            .map(|t| t.detected_source_language.to_ascii_lowercase())
            .unwrap_or_default();
        Ok(Translation { source, texts: body.translations.into_iter().map(|t| t.text).collect() })
    }
}

/// A post's text (and its quote's) in the requested language.
pub struct Translated {
    /// English name of the detected source language, e.g. "Japanese".
    pub source: String,
    pub text: Option<String>,
    pub quote: Option<String>,
}

impl Translator {
    /// Translates the post and quote text in one request. `None` when there is no text or it
    /// is already in the target language.
    pub async fn translate_post(
        &self,
        post: &Post,
        target: &'static str,
    ) -> Result<Option<Translated>, FetchError> {
        if post.lang.as_deref().is_some_and(|l| same_language(l, target)) {
            return Ok(None);
        }
        let text = post.text.as_deref();
        let quote = post.quote.as_ref().and_then(|q| q.text.as_deref());
        let texts: Vec<&str> = text.iter().chain(quote.iter()).copied().collect();
        if texts.is_empty() {
            return Ok(None);
        }
        let Translation { source, texts } = self.translate(&texts, target).await?;
        if same_language(&source, target) {
            return Ok(None);
        }
        let mut texts = texts.into_iter();
        Ok(Some(Translated {
            source: language_name(&source).map_or(source.to_uppercase(), str::to_owned),
            text: text.and_then(|_| texts.next()),
            quote: quote.and_then(|_| texts.next()),
        }))
    }
}

/// DeepL language codes (uppercase) and English names. Includes regional target variants.
const LANGUAGES: &[(&str, &str)] = &[
    ("ACE", "Acehnese"), ("AF", "Afrikaans"), ("SQ", "Albanian"), ("AR", "Arabic"), ("AN", "Aragonese"),
    ("HY", "Armenian"), ("AS", "Assamese"), ("AY", "Aymara"), ("AZ", "Azerbaijani"), ("BA", "Bashkir"),
    ("EU", "Basque"), ("BE", "Belarusian"), ("BN", "Bengali"), ("BHO", "Bhojpuri"), ("BS", "Bosnian"),
    ("BR", "Breton"), ("BG", "Bulgarian"), ("MY", "Burmese"), ("YUE", "Cantonese"), ("CA", "Catalan"),
    ("CEB", "Cebuano"), ("ZH", "Chinese"), ("ZH-HANS", "Chinese (simplified)"),
    ("ZH-HANT", "Chinese (traditional)"), ("HR", "Croatian"), ("CS", "Czech"), ("DA", "Danish"),
    ("PRS", "Dari"), ("NL", "Dutch"), ("EN", "English"), ("EN-US", "English (American)"),
    ("EN-GB", "English (British)"), ("EO", "Esperanto"), ("ET", "Estonian"), ("FI", "Finnish"),
    ("FR", "French"), ("FR-CA", "French (Canadian)"), ("FR-FR", "French (France)"), ("GL", "Galician"),
    ("KA", "Georgian"), ("DE", "German"), ("DE-DE", "German (Germany)"), ("DE-CH", "German (Swiss)"),
    ("EL", "Greek"), ("GN", "Guarani"), ("GU", "Gujarati"), ("HT", "Haitian Creole"), ("HA", "Hausa"),
    ("HE", "Hebrew"), ("HI", "Hindi"), ("HU", "Hungarian"), ("IS", "Icelandic"), ("IG", "Igbo"),
    ("ID", "Indonesian"), ("GA", "Irish"), ("IT", "Italian"), ("JA", "Japanese"), ("JV", "Javanese"),
    ("PAM", "Kapampangan"), ("KK", "Kazakh"), ("GOM", "Konkani"), ("KO", "Korean"),
    ("KMR", "Kurdish (Kurmanji)"), ("CKB", "Kurdish (Sorani)"), ("KY", "Kyrgyz"), ("LA", "Latin"),
    ("LV", "Latvian"), ("LN", "Lingala"), ("LT", "Lithuanian"), ("LMO", "Lombard"), ("LB", "Luxembourgish"),
    ("MK", "Macedonian"), ("MAI", "Maithili"), ("MG", "Malagasy"), ("MS", "Malay"), ("ML", "Malayalam"),
    ("MT", "Maltese"), ("MI", "Maori"), ("MR", "Marathi"), ("MN", "Mongolian"), ("NE", "Nepali"),
    ("NB", "Norwegian (bokmål)"), ("OC", "Occitan"), ("OM", "Oromo"), ("PAG", "Pangasinan"),
    ("PS", "Pashto"), ("FA", "Persian"), ("PL", "Polish"), ("PT", "Portuguese"),
    ("PT-BR", "Portuguese (Brazilian)"), ("PT-PT", "Portuguese (European)"), ("PA", "Punjabi"),
    ("QU", "Quechua"), ("RO", "Romanian"), ("RU", "Russian"), ("SA", "Sanskrit"), ("SR", "Serbian"),
    ("ST", "Sesotho"), ("SCN", "Sicilian"), ("SK", "Slovak"), ("SL", "Slovenian"), ("ES", "Spanish"),
    ("ES-419", "Spanish (Latin American)"), ("SU", "Sundanese"), ("SW", "Swahili"), ("SV", "Swedish"),
    ("TL", "Tagalog"), ("TG", "Tajik"), ("TA", "Tamil"), ("TT", "Tatar"), ("TE", "Telugu"), ("TH", "Thai"),
    ("TS", "Tsonga"), ("TN", "Tswana"), ("TR", "Turkish"), ("TK", "Turkmen"), ("UK", "Ukrainian"),
    ("UR", "Urdu"), ("UZ", "Uzbek"), ("VI", "Vietnamese"), ("CY", "Welsh"), ("WO", "Wolof"),
    ("XH", "Xhosa"), ("YI", "Yiddish"), ("ZU", "Zulu"),
];

/// Normalizes a URL language suffix to a DeepL target language code, or None if unsupported.
///
/// Trims, lowercases and strips `|` (Discord spoiler links append `||`), then applies FxEmbed-style aliases.
pub fn target_code(lang: &str) -> Option<&'static str> {
    let lang: String = lang.trim().chars().filter(|&c| c != '|').collect::<String>().to_ascii_lowercase();
    let aliased = match lang.as_str() {
        "zh" | "cn" | "zh-cn" | "zh-hans" => "zh-hans",
        "tw" | "hk" | "zh-tw" | "zh-hk" | "zh-hant" => "zh-hant",
        "jp" => "ja",
        "kr" => "ko",
        "en" => "en-us",
        "uk-ua" | "ua" => "uk",
        "pt" | "br" => "pt-br",
        "no" | "nn" => "nb",
        "iw" => "he",
        "in" => "id",
        "fil" => "tl",
        other => other,
    };
    LANGUAGES
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(aliased))
        .map(|(code, _)| *code)
        // Unspecified-variant codes are not valid translation targets.
        .filter(|code| !matches!(*code, "EN" | "PT" | "ZH"))
}

/// English name for a lowercase ISO 639-1 code (DeepL source languages), for the "Translated from {name}" header.
pub fn language_name(code: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code.trim())).map(|(_, name)| *name)
}

/// True when the post language (ISO 639-1 as reported by X) already matches the target code.
pub fn same_language(post_lang: &str, target: &'static str) -> bool {
    fn primary(code: &str) -> &str {
        code.split(['-', '_']).next().unwrap_or(code)
    }
    let post = post_lang.trim().to_ascii_lowercase();
    let post = match primary(&post) {
        "iw" => "he",
        "in" => "id",
        other => other,
    };
    post.eq_ignore_ascii_case(primary(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_aliases() {
        assert_eq!(target_code("ja"), Some("JA"));
        assert_eq!(target_code("jp"), Some("JA"));
        assert_eq!(target_code("kr"), Some("KO"));
        assert_eq!(target_code("en"), Some("EN-US"));
        assert_eq!(target_code("en-gb"), Some("EN-GB"));
        assert_eq!(target_code("pt"), Some("PT-BR"));
        assert_eq!(target_code("pt-pt"), Some("PT-PT"));
        for l in ["zh", "cn", "zh-cn", "ZH-Hans"] {
            assert_eq!(target_code(l), Some("ZH-HANS"));
        }
        for l in ["tw", "hk", "zh-tw", "zh-hk", "zh-hant"] {
            assert_eq!(target_code(l), Some("ZH-HANT"));
        }
    }

    #[test]
    fn target_rejections_and_spoilers() {
        assert_eq!(target_code("xx"), None);
        assert_eq!(target_code(""), None);
        assert_eq!(target_code("analytics"), None);
        assert_eq!(target_code("||"), None);
        assert_eq!(target_code("ja||"), Some("JA"));
        assert_eq!(target_code(" ||De|| "), Some("DE"));
    }

    #[test]
    fn names() {
        assert_eq!(language_name("ja"), Some("Japanese"));
        assert_eq!(language_name("zh"), Some("Chinese"));
        assert_eq!(language_name("xx"), None);
    }

    #[test]
    fn same() {
        assert!(same_language("en", "EN-US"));
        assert!(same_language("zh", "ZH-HANS"));
        assert!(same_language("zh", "ZH-HANT"));
        assert!(!same_language("ja", "EN-US"));
        assert!(!same_language("", "EN-US"));
    }

    #[test]
    fn endpoint() {
        let c = reqwest::Client::new();
        assert_eq!(Translator::new(c.clone(), "abc:fx".into()).endpoint, FREE_ENDPOINT);
        assert_eq!(Translator::new(c, "abc".into()).endpoint, PRO_ENDPOINT);
    }
}
