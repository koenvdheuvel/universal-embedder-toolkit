package dev.uet.embedfix

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class RewriterTest {
    private val base = "https://fix.example.com"

    private fun rw(url: String) = Rewriter.rewrite(url, base)

    @Test fun reelsPath() =
        assertEquals("$base/reels/DeMPFloOrjv/", rw("https://www.instagram.com/reels/DeMPFloOrjv/"))

    @Test fun imgIndexKeptTrackingDropped() =
        assertEquals("$base/p/ABC123/?img_index=3", rw("https://instagram.com/p/ABC123/?img_index=3&igsh=xyz"))

    @Test fun imgIndexAfterTracking() =
        assertEquals("$base/p/ABC123/?img_index=2", rw("https://instagram.com/p/ABC123/?igsh=xyz&img_index=2"))

    @Test fun usernamePrefixedReel() =
        assertEquals("$base/someuser/reel/ABC_-1/", rw("https://www.instagram.com/someuser/reel/ABC_-1/"))

    @Test fun profileDoesNotMatch() = assertNull(rw("https://www.instagram.com/someuser/"))

    @Test fun audioShortcodeRejected() = assertNull(rw("https://www.instagram.com/reels/audio/123/"))

    @Test fun foreignHostRejected() = assertNull(rw("https://example.com/p/ABC/"))

    @Test fun lookalikeHostRejected() {
        assertNull(rw("https://notinstagram.com/p/ABC/"))
        assertNull(rw("https://instagram.com.evil.test/p/ABC/"))
    }

    @Test fun hostIsCaseInsensitive() =
        assertEquals("$base/p/ABC/", rw("https://WWW.Instagram.COM/p/ABC/"))

    @Test fun mobileHost() = assertEquals("$base/p/ABC/", rw("https://m.instagram.com/p/ABC/"))

    @Test fun shortDomains() {
        assertEquals("$base/p/ABC/", rw("https://instagr.am/p/ABC/"))
        assertEquals("$base/p/ABC/", rw("https://www.instagr.am/p/ABC/"))
    }

    @Test fun shareShortLink() =
        assertEquals("$base/share/reel/xyz/", rw("https://www.instagram.com/share/reel/xyz/"))

    @Test fun shareNeedsSecondSegment() = assertNull(rw("https://www.instagram.com/share/"))

    @Test fun tvPath() = assertEquals("$base/tv/X/", rw("https://www.instagram.com/tv/X/"))

    @Test fun pathWithoutTrailingSlashKeptVerbatim() =
        assertEquals("$base/p/ABC", rw("https://www.instagram.com/p/ABC"))

    @Test fun fragmentDropped() = assertEquals("$base/p/ABC/", rw("https://www.instagram.com/p/ABC/#comments"))

    @Test fun invalidImgIndexDropped() {
        for (v in listOf("0", "-1", "abc", "", "1.5", "+2", "99999999999999999999")) {
            assertEquals("img_index=$v", "$base/p/ABC/", rw("https://www.instagram.com/p/ABC/?img_index=$v"))
        }
    }

    @Test fun httpSchemeAccepted() = assertEquals("$base/p/ABC/", rw("http://www.instagram.com/p/ABC/"))

    @Test fun nonHttpSchemeRejected() {
        assertNull(rw("ftp://www.instagram.com/p/ABC/"))
        assertNull(rw("javascript://www.instagram.com/p/ABC/"))
        assertNull(rw("www.instagram.com/p/ABC/"))
    }

    @Test fun inputTrimmed() = assertEquals("$base/p/ABC/", rw("  https://www.instagram.com/p/ABC/\n"))

    @Test fun garbageRejected() {
        assertNull(rw(""))
        assertNull(rw("not a url"))
        assertNull(rw("https://"))
    }

    @Test fun invalidShortcodeRejected() {
        assertNull(rw("https://www.instagram.com/p/a%20b/"))
        assertNull(rw("https://www.instagram.com/p/${"a".repeat(65)}/"))
        assertEquals("$base/p/${"a".repeat(64)}/", rw("https://www.instagram.com/p/${"a".repeat(64)}/"))
    }

    @Test fun fixBaseTrailingSlashTolerated() =
        assertEquals("$base/p/ABC/", Rewriter.rewrite("https://www.instagram.com/p/ABC/", "$base/"))

    @Test fun findAndRewriteInSurroundingText() {
        val text = "Check this out https://www.instagram.com/reel/X1y/?igsh=MWx4 so good"
        assertEquals("$base/reel/X1y/", Rewriter.findAndRewrite(text, base))
    }

    @Test fun findSkipsNonInstagramUrls() {
        val text = "see https://example.com/a and https://www.instagram.com/p/ABC/?img_index=2 ok"
        assertEquals("$base/p/ABC/?img_index=2", Rewriter.findAndRewrite(text, base))
    }

    @Test fun findSkipsNonMatchingInstagramUrls() {
        val text = "https://www.instagram.com/someuser/ then https://www.instagram.com/p/ABC/"
        assertEquals("$base/p/ABC/", Rewriter.findAndRewrite(text, base))
    }

    @Test fun xQueryDropped() = assertEquals(
        "$base/elonmusk/status/1585341984679469056",
        rw("https://x.com/elonmusk/status/1585341984679469056?s=46&t=abc"),
    )

    @Test fun xMobileTwitterPhotoKept() = assertEquals(
        "$base/nasa/status/2050014959552114958/photo/3",
        rw("https://Mobile.Twitter.com/nasa/status/2050014959552114958/photo/3"),
    )

    @Test fun xIWeb() = assertEquals("$base/i/web/status/20", rw("https://twitter.com/i/web/status/20"))
    @Test fun xStatuses() = assertEquals("$base/a/statuses/5", rw("https://www.x.com/a/statuses/5"))
    @Test fun xHostCaseInsensitive() = assertEquals("$base/a/status/5", rw("https://WWW.X.COM/a/status/5"))
    @Test fun xUserNamedStatus() = assertEquals("$base/status/status/20", rw("https://x.com/status/status/20"))
    @Test fun xProfileRejected() = assertNull(rw("https://x.com/elonmusk"))
    @Test fun xInstagramPathRejected() = assertNull(rw("https://x.com/p/ABC/"))
    @Test fun instagramHostXPathRejected() = assertNull(rw("https://www.instagram.com/jack/status/20"))
    @Test fun xIdTooLongRejected() = assertNull(rw("https://x.com/a/status/123456789012345678901"))
    @Test fun xIdMaxLenAccepted() =
        assertEquals("$base/a/status/12345678901234567890", rw("https://x.com/a/status/12345678901234567890"))
    @Test fun xNonDigitIdRejected() = assertNull(rw("https://x.com/a/status/12a"))

    @Test fun findXInShareText() = assertEquals(
        "$base/a/status/1",
        Rewriter.findAndRewrite("Look https://x.com/a/status/1?s=20 lol", base),
    )

    @Test fun findStripsTrailingPunctuation() {
        assertEquals("$base/p/ABC/", Rewriter.findAndRewrite("(https://www.instagram.com/p/ABC/).", base))
        assertEquals("$base/p/ABC", Rewriter.findAndRewrite("look: https://www.instagram.com/p/ABC.", base))
    }

    @Test fun findNoMatch() {
        assertNull(Rewriter.findAndRewrite("nothing here", base))
        assertNull(Rewriter.findAndRewrite("https://www.instagram.com/someuser/", base))
        assertNull(Rewriter.findAndRewrite("", base))
    }

    @Test fun findReportsRangeForInPlaceReplacement() {
        val text = "hey https://www.instagram.com/p/ABC/?igsh=1 bye"
        val m = Rewriter.find(text, base)
        assertNotNull(m)
        val replaced = text.substring(0, m!!.start) + m.rewritten + text.substring(m.end)
        assertEquals("hey $base/p/ABC/ bye", replaced)
    }

    @Test fun normalizeBase() {
        assertEquals("https://my.host", Rewriter.normalizeBase("  my.host/ "))
        assertEquals("https://my.host", Rewriter.normalizeBase("https://my.host///"))
        assertEquals("http://my.host:8080/x", Rewriter.normalizeBase("http://my.host:8080/x/"))
        assertNull(Rewriter.normalizeBase(""))
        assertNull(Rewriter.normalizeBase("   "))
        assertNull(Rewriter.normalizeBase("ftp://my.host"))
        assertNull(Rewriter.normalizeBase("https://my.host/?a=b"))
        assertNull(Rewriter.normalizeBase("https://"))
    }
}
