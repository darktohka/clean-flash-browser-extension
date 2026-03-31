package org.cleanflash.android.input

import android.view.MotionEvent
import org.cleanflash.android.ipc.MessageCodec

/**
 * Translates Android touch events to Flash mouse events.
 */
class TouchHandler(
    private val sendMessage: (tag: Int, payload: ByteArray) -> Unit
) {
    /**
     * Handle a touch event from the overlay view.
     *
     * Maps touch coordinates to Flash logical coordinates, accounting for
     * aspect-ratio letterboxing via [viewportOffsetX] / [viewportOffsetY].
     */
    fun onTouchEvent(event: MotionEvent, flashWidth: Int, flashHeight: Int,
                     viewportWidth: Int, viewportHeight: Int,
                     viewportOffsetX: Int = 0, viewportOffsetY: Int = 0) {
        // Adjust for letterbox/pillarbox offset
        val adjustedX = event.x - viewportOffsetX
        val adjustedY = event.y - viewportOffsetY

        val scaleX = flashWidth.toFloat() / viewportWidth.coerceAtLeast(1)
        val scaleY = flashHeight.toFloat() / viewportHeight.coerceAtLeast(1)

        val flashX = (adjustedX * scaleX).toInt().coerceIn(0, flashWidth)
        val flashY = (adjustedY * scaleY).toInt().coerceIn(0, flashHeight)

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                // Send mouse enter + mouse down (left button)
                sendMouseEvent(MessageCodec.TAG_MOUSE_ENTER, 0, 0, 0, 0)
                sendMouseEvent(MessageCodec.TAG_MOUSE_DOWN, flashX, flashY, 0, 0) // button 0 = left
            }

            MotionEvent.ACTION_MOVE -> {
                sendMouseEvent(MessageCodec.TAG_MOUSE_MOVE, flashX, flashY, 0, 0)
            }

            MotionEvent.ACTION_UP -> {
                sendMouseEvent(MessageCodec.TAG_MOUSE_UP, flashX, flashY, 0, 0)
            }

            MotionEvent.ACTION_CANCEL -> {
                sendMouseEvent(MessageCodec.TAG_MOUSE_UP, flashX, flashY, 0, 0)
                sendMouseEvent(MessageCodec.TAG_MOUSE_LEAVE, 0, 0, 0, 0)
            }
        }
    }

    private fun sendMouseEvent(tag: Int, x: Int, y: Int, button: Int, modifiers: Int) {
        val pw = MessageCodec.PayloadWriter()
        pw.writeU32(x)
        pw.writeU32(y)
        pw.writeU8(button)
        pw.writeU32(modifiers)
        sendMessage(tag, pw.finish())
    }
}
