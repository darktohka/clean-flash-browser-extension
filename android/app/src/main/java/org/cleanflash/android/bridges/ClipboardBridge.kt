package org.cleanflash.android.bridges

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import org.cleanflash.android.ipc.MessageCodec

/**
 * Android clipboard access bridge.
 */
object ClipboardBridge {

    /**
     * Read from the system clipboard.
     */
    fun readClipboard(context: Context, payload: ByteArray): ByteArray {
        val reader = MessageCodec.PayloadReader(payload)
        val format = reader.readU8() // 0=plain, 1=html, 2=rtf

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val pw = MessageCodec.PayloadWriter()

        if (!clipboard.hasPrimaryClip()) {
            pw.writeU8(0) // no data
            return pw.finish()
        }

        val clip = clipboard.primaryClip ?: run {
            pw.writeU8(0)
            return pw.finish()
        }

        val item = clip.getItemAt(0)
        val text = when (format) {
            0 -> item.text?.toString()
            1 -> item.htmlText
            else -> null
        }

        if (text != null) {
            pw.writeU8(1) // has data
            pw.writeString(text)
        } else {
            pw.writeU8(0)
        }

        return pw.finish()
    }

    /**
     * Write to the system clipboard.
     */
    fun writeClipboard(context: Context, payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val count = reader.readU32()

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

        for (i in 0 until count) {
            val format = reader.readU8()
            val data = reader.readBytes()

            when (format) {
                0 -> { // Plain text
                    val text = String(data, Charsets.UTF_8)
                    clipboard.setPrimaryClip(ClipData.newPlainText("Flash", text))
                }
                1 -> { // HTML
                    val html = String(data, Charsets.UTF_8)
                    clipboard.setPrimaryClip(ClipData.newHtmlText("Flash", html, html))
                }
            }
        }
    }
}
