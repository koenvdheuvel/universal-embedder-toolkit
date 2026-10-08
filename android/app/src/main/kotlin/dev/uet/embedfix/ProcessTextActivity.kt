package dev.uet.embedfix

import android.app.Activity
import android.content.Intent
import android.os.Bundle

class ProcessTextActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val text = intent?.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString().orEmpty()
        val readOnly = intent?.getBooleanExtra(Intent.EXTRA_PROCESS_TEXT_READONLY, false) ?: false
        val fixBase = Settings.fixBase(this)

        val match = Rewriter.find(text, fixBase)
        if (match == null) {
            Clip.toast(this, R.string.toast_no_link)
            setResult(RESULT_CANCELED)
        } else if (readOnly) {
            Clip.copy(this, match.rewritten)
            Clip.toastCopied(this)
            setResult(RESULT_CANCELED)
        } else {
            val replaced = text.substring(0, match.start) + match.rewritten + text.substring(match.end)
            setResult(RESULT_OK, Intent().putExtra(Intent.EXTRA_PROCESS_TEXT, replaced))
        }
        finish()
    }
}
