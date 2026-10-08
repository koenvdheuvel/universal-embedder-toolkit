package dev.uet.embedfix

import android.app.Activity
import android.content.Intent
import android.os.Bundle

class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val text = intent?.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty()
        val fixed = Rewriter.findAndRewrite(text, Settings.fixBase(this))
        if (fixed == null) {
            Clip.toast(this, R.string.toast_no_link)
        } else {
            Clip.copy(this, fixed)
            Clip.toastCopied(this)
        }
        finish()
    }
}
