package dev.uet.embedfix

import android.content.Context

object Settings {
    private const val PREFS = "settings"
    private const val KEY_FIX_BASE = "fix_base_url"

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Stored fix base URL, falling back to the build-time default. */
    fun fixBase(ctx: Context): String =
        prefs(ctx).getString(KEY_FIX_BASE, null)?.let(Rewriter::normalizeBase)
            ?: Rewriter.normalizeBase(BuildConfig.FIX_BASE_URL)
            ?: "https://fix.example.com"

    /** Normalizes and stores [input]; returns the stored value, or null if [input] is invalid. */
    fun saveFixBase(ctx: Context, input: String): String? {
        val normalized = Rewriter.normalizeBase(input) ?: return null
        prefs(ctx).edit().putString(KEY_FIX_BASE, normalized).apply()
        return normalized
    }
}
