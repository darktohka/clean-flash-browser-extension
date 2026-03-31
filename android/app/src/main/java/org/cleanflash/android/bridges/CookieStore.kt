package org.cleanflash.android.bridges

import android.content.Context
import android.content.SharedPreferences
import org.cleanflash.android.ipc.MessageCodec

/**
 * Simple cookie store backed by SharedPreferences.
 *
 * Stores cookies as semicolon-separated key=value pairs per domain.
 * A production version should use SQLite with full cookie parsing.
 */
object CookieStore {

    private fun getPrefs(context: Context): SharedPreferences {
        return context.getSharedPreferences("flash_cookies", Context.MODE_PRIVATE)
    }

    private fun domainFromUrl(url: String): String {
        return try {
            val uri = java.net.URI(url)
            uri.host ?: "unknown"
        } catch (e: Exception) {
            "unknown"
        }
    }

    /**
     * Get cookies for a URL.
     */
    fun getCookies(context: Context, payload: ByteArray): ByteArray {
        val reader = MessageCodec.PayloadReader(payload)
        val url = reader.readString()
        val domain = domainFromUrl(url)

        val cookies = getPrefs(context).getString("cookies_$domain", null)
        val pw = MessageCodec.PayloadWriter()

        if (cookies != null && cookies.isNotEmpty()) {
            pw.writeU8(1)
            pw.writeString(cookies)
        } else {
            pw.writeU8(0)
        }

        return pw.finish()
    }

    /**
     * Set cookies from HTTP response headers.
     */
    fun setCookies(context: Context, payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val url = reader.readString()
        val domain = domainFromUrl(url)
        val count = reader.readU32()

        val prefs = getPrefs(context)
        val existing = prefs.getString("cookies_$domain", "") ?: ""
        val cookieMap = LinkedHashMap<String, String>()

        // Parse existing cookies
        for (part in existing.split("; ")) {
            val eq = part.indexOf('=')
            if (eq > 0) {
                cookieMap[part.substring(0, eq).trim()] = part.substring(eq + 1).trim()
            }
        }

        // Parse new Set-Cookie headers
        for (i in 0 until count) {
            val header = reader.readString()
            // Simple parsing: take name=value before first semicolon
            val nameValue = header.substringBefore(";").trim()
            val eq = nameValue.indexOf('=')
            if (eq > 0) {
                cookieMap[nameValue.substring(0, eq).trim()] = nameValue.substring(eq + 1).trim()
            }
        }

        // Rebuild cookie string
        val cookieString = cookieMap.entries.joinToString("; ") { "${it.key}=${it.value}" }
        prefs.edit().putString("cookies_$domain", cookieString).apply()
    }
}
