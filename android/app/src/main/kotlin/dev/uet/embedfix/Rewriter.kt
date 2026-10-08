package dev.uet.embedfix

import java.net.URI

/**
 * Pure URL rewriting logic (no Android APIs). Mirrors the server's path scheme:
 * `https://www.instagram.com/reels/X/` -> `<fixBase>/reels/X/` and
 * `https://x.com/user/status/123?s=20` -> `<fixBase>/user/status/123`.
 */
object Rewriter {
    /** A rewritten URL found inside free text; [start] inclusive, [end] exclusive. */
    data class Match(val start: Int, val end: Int, val original: String, val rewritten: String)

    private val HOSTS = setOf(
        "instagram.com",
        "www.instagram.com",
        "m.instagram.com",
        "instagr.am",
        "www.instagr.am",
    )
    private val TWITTER_HOSTS = setOf(
        "x.com",
        "www.x.com",
        "mobile.x.com",
        "twitter.com",
        "www.twitter.com",
        "mobile.twitter.com",
    )
    private val TWEET_ID = Regex("[0-9]{1,20}")
    private val KINDS = setOf("p", "reel", "reels", "tv")
    private val SHORTCODE = Regex("[A-Za-z0-9_-]{1,64}")
    private val SCHEME_PREFIX = Regex("^[A-Za-z][A-Za-z0-9+.-]*://")
    private val DIGITS = Regex("[0-9]+")
    private val URL_IN_TEXT = Regex("https?://[^\\s<>\"']+", RegexOption.IGNORE_CASE)
    private const val TRAILING_PUNCT = ".,;:!?)]}"

    /** Rewrites a single Instagram or X/Twitter URL; null when [url] is not a supported link. */
    fun rewrite(url: String, fixBase: String): String? {
        val uri = try {
            URI(url.trim())
        } catch (_: Exception) {
            return null
        }
        val scheme = uri.scheme?.lowercase() ?: return null
        if (scheme != "http" && scheme != "https") return null
        val host = uri.host?.lowercase() ?: return null
        val twitter = host in TWITTER_HOSTS
        if (!twitter && host !in HOSTS) return null

        val rawPath = uri.rawPath ?: ""
        val segments = rawPath.split('/').filter { it.isNotEmpty() }
        if (!(if (twitter) matchesTwitterPath(segments) else matchesPath(segments))) return null

        val sb = StringBuilder(fixBase.trimEnd('/')).append(rawPath)
        if (!twitter) imgIndex(uri.rawQuery)?.let { sb.append("?img_index=").append(it) }
        return sb.toString()
    }

    /** Finds the first Instagram or X/Twitter URL in [text] that can be rewritten. */
    fun find(text: String, fixBase: String): Match? {
        for (m in URL_IN_TEXT.findAll(text)) {
            var end = m.range.last + 1
            while (end > m.range.first && text[end - 1] in TRAILING_PUNCT) end--
            val candidate = text.substring(m.range.first, end)
            val rewritten = rewrite(candidate, fixBase) ?: continue
            return Match(m.range.first, end, candidate, rewritten)
        }
        return null
    }

    /** Extracts the first matching Instagram or X/Twitter URL from arbitrary [text] and rewrites it. */
    fun findAndRewrite(text: String, fixBase: String): String? = find(text, fixBase)?.rewritten

    /**
     * Normalizes user input for the fix base URL: trim, strip trailing slashes,
     * prepend `https://` when no scheme is given. Null if the result is not a usable http(s) base.
     */
    fun normalizeBase(input: String): String? {
        var s = input.trim()
        if (s.isEmpty()) return null
        val explicit = SCHEME_PREFIX.find(s)?.value
        val rest = if (explicit != null) s.substring(explicit.length) else s
        s = (explicit ?: "https://") + rest.trimEnd('/')
        if ('?' in s || '#' in s) return null
        val uri = try {
            URI(s)
        } catch (_: Exception) {
            return null
        }
        val scheme = uri.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") return null
        if (uri.host.isNullOrEmpty()) return null
        return s
    }

    private fun isShortcode(s: String) = s != "audio" && SHORTCODE.matches(s)

    private fun matchesTwitterPath(seg: List<String>): Boolean {
        val rest = when {
            seg.size >= 4 && seg[0] == "i" && seg[1] == "web" -> seg.subList(2, seg.size)
            seg.size >= 3 -> seg.subList(1, seg.size)
            else -> return false
        }
        return (rest[0] == "status" || rest[0] == "statuses") &&
            rest.size >= 2 && TWEET_ID.matches(rest[1])
    }

    private fun matchesPath(seg: List<String>): Boolean {
        if (seg.size >= 2 && seg[0] == "share") return true
        if (seg.size >= 2 && seg[0] in KINDS && isShortcode(seg[1])) return true
        if (seg.size >= 3 && seg[1] in KINDS && isShortcode(seg[2])) return true
        return false
    }

    private fun imgIndex(rawQuery: String?): Long? {
        if (rawQuery.isNullOrEmpty()) return null
        val value = rawQuery.split('&')
            .firstOrNull { it.substringBefore('=') == "img_index" && '=' in it }
            ?.substringAfter('=')
            ?: return null
        if (!DIGITS.matches(value)) return null
        return value.toLongOrNull()?.takeIf { it > 0 }
    }
}
