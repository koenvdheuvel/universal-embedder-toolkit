package dev.uet.embedfix

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.widget.Toast

internal object Clip {
    fun copy(ctx: Context, text: String) {
        val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("Fixed link", text))
    }

    /** Android 13+ shows its own clipboard confirmation. */
    fun toastCopied(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            Toast.makeText(ctx, R.string.toast_fixed, Toast.LENGTH_SHORT).show()
        }
    }

    fun toast(ctx: Context, resId: Int) {
        Toast.makeText(ctx, resId, Toast.LENGTH_SHORT).show()
    }
}
