package dev.uet.embedfix

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context

/**
 * Launched by [FixClipboardTileService]. Since Android 10 only the focused app may read the
 * clipboard, so the work happens once this window gains focus.
 */
class ClipboardFixActivity : Activity() {
    private var handled = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || handled) return
        handled = true

        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clip = cm.primaryClip
        val text = if (clip != null && clip.itemCount > 0) {
            clip.getItemAt(0).coerceToText(this)?.toString().orEmpty()
        } else {
            ""
        }

        val fixed = Rewriter.findAndRewrite(text, Settings.fixBase(this))
        if (fixed == null) {
            Clip.toast(this, R.string.toast_no_link_clipboard)
        } else {
            Clip.copy(this, fixed)
            Clip.toastCopied(this)
        }
        finish()
    }
}
