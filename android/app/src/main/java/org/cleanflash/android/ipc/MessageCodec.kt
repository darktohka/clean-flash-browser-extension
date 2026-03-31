package org.cleanflash.android.ipc

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Binary protocol encoder/decoder for IPC messages.
 *
 * Matches the Rust-side protocol module in player-android.
 */
object MessageCodec {

    // ---- Tag constants: Android → Host ----
    const val TAG_OPEN = 0x01
    const val TAG_CLOSE = 0x02
    const val TAG_RESIZE = 0x03
    const val TAG_VIEW_UPDATE = 0x04

    const val TAG_MOUSE_DOWN = 0x10
    const val TAG_MOUSE_UP = 0x11
    const val TAG_MOUSE_MOVE = 0x12
    const val TAG_MOUSE_ENTER = 0x13
    const val TAG_MOUSE_LEAVE = 0x14
    const val TAG_WHEEL = 0x15

    const val TAG_KEY_DOWN = 0x20
    const val TAG_KEY_UP = 0x21
    const val TAG_KEY_CHAR = 0x22
    const val TAG_IME_COMPOSITION_START = 0x23
    const val TAG_IME_COMPOSITION_UPDATE = 0x24
    const val TAG_IME_COMPOSITION_END = 0x25

    const val TAG_FOCUS = 0x30

    const val TAG_HTTP_RESPONSE = 0x40
    const val TAG_AUDIO_INPUT_DATA = 0x41
    const val TAG_VIDEO_CAPTURE_DATA = 0x42
    const val TAG_MENU_RESPONSE = 0x43
    const val TAG_CLIPBOARD_RESPONSE = 0x44
    const val TAG_COOKIE_RESPONSE = 0x45
    const val TAG_DIALOG_RESPONSE = 0x46
    const val TAG_FILE_CHOOSER_RESPONSE = 0x47
    const val TAG_SETTINGS_UPDATE = 0x48

    // ---- Tag constants: Host → Android ----
    const val TAG_FRAME_READY = 0x80
    const val TAG_FRAME_INIT = 0x81
    const val TAG_STATE_CHANGE = 0x82
    const val TAG_CURSOR_CHANGE = 0x83
    const val TAG_NAVIGATE = 0x84

    const val TAG_AUDIO_INIT = 0x90
    const val TAG_AUDIO_START = 0x91
    const val TAG_AUDIO_STOP = 0x92
    const val TAG_AUDIO_CLOSE = 0x93
    const val TAG_AUDIO_SAMPLES = 0x94

    const val TAG_HTTP_REQUEST = 0xC0
    const val TAG_CLIPBOARD_READ = 0xC1
    const val TAG_CLIPBOARD_WRITE = 0xC2
    const val TAG_COOKIE_GET = 0xC3
    const val TAG_COOKIE_SET = 0xC4
    const val TAG_CONTEXT_MENU_SHOW = 0xC5
    const val TAG_DIALOG_SHOW = 0xC6
    const val TAG_FILE_CHOOSER_SHOW = 0xC7
    const val TAG_FULLSCREEN_SET = 0xC8
    const val TAG_FULLSCREEN_QUERY = 0xC9

    const val TAG_VERSION = 0xD0

    /**
     * Build the Open command payload.
     */
    fun buildOpenCommand(url: String, width: Int, height: Int): ByteArray {
        val pw = PayloadWriter()
        pw.writeString(url)
        pw.writeU32(width)
        pw.writeU32(height)
        pw.writeString("{}") // settings JSON
        return pw.finish()
    }

    /**
     * Helper for building binary payloads.
     */
    class PayloadWriter {
        private val buffer = ByteBuffer.allocate(65536).order(ByteOrder.LITTLE_ENDIAN)

        fun writeU8(v: Int) { buffer.put(v.toByte()) }
        fun writeU16(v: Int) { buffer.putShort(v.toShort()) }
        fun writeU32(v: Int) { buffer.putInt(v) }
        fun writeI32(v: Int) { buffer.putInt(v) }
        fun writeF32(v: Float) { buffer.putFloat(v) }

        fun writeString(s: String) {
            val bytes = s.toByteArray(Charsets.UTF_8)
            buffer.putInt(bytes.size)
            buffer.put(bytes)
        }

        fun writeBytes(data: ByteArray) {
            buffer.putInt(data.size)
            buffer.put(data)
        }

        fun finish(): ByteArray {
            val result = ByteArray(buffer.position())
            buffer.flip()
            buffer.get(result)
            return result
        }
    }

    /**
     * Helper for reading binary payloads.
     */
    class PayloadReader(private val data: ByteArray) {
        private val buffer = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN)

        fun remaining(): Int = buffer.remaining()

        fun readU8(): Int = buffer.get().toInt() and 0xFF
        fun readU16(): Int = buffer.short.toInt() and 0xFFFF
        fun readU32(): Int = buffer.int
        fun readI32(): Int = buffer.int
        fun readF32(): Float = buffer.float

        fun readString(): String {
            val len = buffer.int
            val bytes = ByteArray(len)
            buffer.get(bytes)
            return String(bytes, Charsets.UTF_8)
        }

        fun readBytes(): ByteArray {
            val len = buffer.int
            val bytes = ByteArray(len)
            buffer.get(bytes)
            return bytes
        }

        fun readRemaining(): ByteArray {
            val bytes = ByteArray(buffer.remaining())
            buffer.get(bytes)
            return bytes
        }
    }
}
