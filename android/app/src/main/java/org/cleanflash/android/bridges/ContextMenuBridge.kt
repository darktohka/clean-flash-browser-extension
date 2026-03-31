package org.cleanflash.android.bridges

import android.app.Activity
import android.app.AlertDialog
import android.view.Gravity
import android.widget.PopupMenu
import org.cleanflash.android.ipc.MessageCodec

/**
 * Context menu bridge — displays Flash right-click menus as Android popups.
 */
object ContextMenuBridge {

    data class MenuItem(
        val type: Int,
        val name: String,
        val id: Int,
        val enabled: Boolean,
        val checked: Boolean,
        val submenu: List<MenuItem>
    )

    /**
     * Show a context menu and invoke the callback with the selected item ID.
     */
    fun showMenu(activity: Activity, payload: ByteArray, callback: (Int) -> Unit) {
        val reader = MessageCodec.PayloadReader(payload)
        val x = reader.readI32()
        val y = reader.readI32()
        val items = readMenuItems(reader)

        if (items.isEmpty()) {
            callback(-1)
            return
        }

        // Use AlertDialog for simple menu display
        val names = items.filter { it.type != 2 } // exclude separators
            .map { it.name }
            .toTypedArray()
        val ids = items.filter { it.type != 2 }
            .map { it.id }
            .toIntArray()

        AlertDialog.Builder(activity)
            .setItems(names) { _, which ->
                callback(ids[which])
            }
            .setOnCancelListener {
                callback(-1)
            }
            .show()
    }

    private fun readMenuItems(reader: MessageCodec.PayloadReader): List<MenuItem> {
        val count = reader.readU32()
        val items = mutableListOf<MenuItem>()
        for (i in 0 until count) {
            val type = reader.readU8()
            val name = reader.readString()
            val id = reader.readI32()
            val enabled = reader.readU8() != 0
            val checked = reader.readU8() != 0
            val submenu = readMenuItems(reader)
            items.add(MenuItem(type, name, id, enabled, checked, submenu))
        }
        return items
    }
}
