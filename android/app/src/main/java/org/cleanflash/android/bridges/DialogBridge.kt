package org.cleanflash.android.bridges

import android.app.Activity
import android.app.AlertDialog
import android.widget.EditText
import org.cleanflash.android.ipc.MessageCodec

/**
 * Dialog bridge — shows alert/confirm/prompt dialogs for Flash content.
 */
object DialogBridge {

    /**
     * Show a dialog based on the type specified in the payload.
     */
    fun showDialog(activity: Activity, payload: ByteArray, callback: (ByteArray) -> Unit) {
        val reader = MessageCodec.PayloadReader(payload)
        val type = reader.readU8() // 0=alert, 1=confirm, 2=prompt
        val message = reader.readString()
        val defaultValue = reader.readString()

        when (type) {
            0 -> showAlert(activity, message, callback)
            1 -> showConfirm(activity, message, callback)
            2 -> showPrompt(activity, message, defaultValue, callback)
            else -> callback(byteArrayOf(0))
        }
    }

    private fun showAlert(activity: Activity, message: String, callback: (ByteArray) -> Unit) {
        AlertDialog.Builder(activity)
            .setMessage(message)
            .setPositiveButton("OK") { _, _ ->
                callback(byteArrayOf(1))
            }
            .setOnCancelListener {
                callback(byteArrayOf(1))
            }
            .show()
    }

    private fun showConfirm(activity: Activity, message: String, callback: (ByteArray) -> Unit) {
        AlertDialog.Builder(activity)
            .setMessage(message)
            .setPositiveButton("OK") { _, _ ->
                callback(byteArrayOf(1))
            }
            .setNegativeButton("Cancel") { _, _ ->
                callback(byteArrayOf(0))
            }
            .setOnCancelListener {
                callback(byteArrayOf(0))
            }
            .show()
    }

    private fun showPrompt(activity: Activity, message: String,
                           defaultValue: String, callback: (ByteArray) -> Unit) {
        val editText = EditText(activity).apply {
            setText(defaultValue)
            setPadding(48, 16, 48, 16)
        }

        AlertDialog.Builder(activity)
            .setMessage(message)
            .setView(editText)
            .setPositiveButton("OK") { _, _ ->
                val input = editText.text.toString()
                val pw = MessageCodec.PayloadWriter()
                pw.writeU8(1) // not cancelled
                pw.writeString(input)
                callback(pw.finish())
            }
            .setNegativeButton("Cancel") { _, _ ->
                val pw = MessageCodec.PayloadWriter()
                pw.writeU8(0) // cancelled
                callback(pw.finish())
            }
            .setOnCancelListener {
                val pw = MessageCodec.PayloadWriter()
                pw.writeU8(0)
                callback(pw.finish())
            }
            .show()
    }
}
