package org.cleanflash.android.ipc

import android.util.Log
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.net.ServerSocket

/**
 * IPC server using AF_UNIX domain sockets for communication with flash-host.
 *
 * Manages the listening socket, client connection, and message dispatch.
 */
class IpcServer(private val socketPath: String) {

    companion object {
        private const val TAG = "IpcServer"

        init {
            System.loadLibrary("flashplayer")
        }
    }

    private var listenFd: Int = -1
    private var clientFd: Int = -1
    private var readerThread: Thread? = null
    private var running = false
    private var messageHandler: ((Int, Int, ByteArray) -> Unit)? = null

    // Synchronized output stream for sending to host
    private val writeLock = Object()

    fun setMessageHandler(handler: (tag: Int, reqId: Int, payload: ByteArray) -> Unit) {
        messageHandler = handler
    }

    /**
     * Start the IPC server: create the listening Unix socket.
     */
    fun start() {
        listenFd = nativeCreateSocket(socketPath)
        if (listenFd < 0) {
            throw IOException("Failed to create IPC socket at $socketPath")
        }
        running = true
        Log.i(TAG, "IPC server started on $socketPath")
    }

    /**
     * Wait for the flash-host to connect (blocking).
     */
    fun waitForConnection(timeoutMs: Long) {
        // Simple blocking accept — could add timeout with poll()
        clientFd = nativeAccept(listenFd)
        if (clientFd < 0) {
            throw IOException("Failed to accept IPC connection")
        }
        Log.i(TAG, "Host connected (fd=$clientFd)")

        // Start reader thread
        readerThread = Thread({
            readLoop()
        }, "ipc-reader").also { it.isDaemon = true; it.start() }
    }

    /**
     * Send a fire-and-forget message to the host.
     */
    fun sendToHost(tag: Int, payload: ByteArray) {
        if (clientFd < 0) return
        val msg = encodeMessage(tag, 0, payload)
        synchronized(writeLock) {
            nativeWriteExact(clientFd, msg, 0, msg.size)
        }
    }

    /**
     * Send a response to a host request (with matching reqId).
     */
    fun sendResponse(tag: Int, reqId: Int, payload: ByteArray) {
        if (clientFd < 0) return
        val msg = encodeMessage(tag, reqId, payload)
        synchronized(writeLock) {
            nativeWriteExact(clientFd, msg, 0, msg.size)
        }
    }

    /**
     * Stop the server and close all connections.
     */
    fun stop() {
        running = false
        if (clientFd >= 0) {
            nativeClose(clientFd)
            clientFd = -1
        }
        if (listenFd >= 0) {
            nativeClose(listenFd)
            listenFd = -1
        }
        readerThread?.interrupt()
    }

    // ---- Internal ----

    private fun readLoop() {
        val header = ByteArray(9) // 4 (length) + 1 (tag) + 4 (reqId)
        try {
            while (running && clientFd >= 0) {
                // Read header
                if (nativeReadExact(clientFd, header, 0, 9) < 0) {
                    Log.i(TAG, "Host disconnected")
                    break
                }

                val length = readU32LE(header, 0)
                val tag = header[4].toInt() and 0xFF
                val reqId = readU32LE(header, 5)

                // Read payload
                val payloadLen = length - 5 // tag(1) + reqId(4) = 5
                val payload = if (payloadLen > 0) {
                    val buf = ByteArray(payloadLen)
                    if (nativeReadExact(clientFd, buf, 0, payloadLen) < 0) {
                        Log.e(TAG, "Failed to read payload")
                        break
                    }
                    buf
                } else {
                    ByteArray(0)
                }

                // Dispatch
                try {
                    messageHandler?.invoke(tag, reqId, payload)
                } catch (e: Exception) {
                    Log.e(TAG, "Error handling message tag=0x${tag.toString(16)}", e)
                }
            }
        } catch (e: Exception) {
            if (running) {
                Log.e(TAG, "Read loop error", e)
            }
        }
    }

    private fun encodeMessage(tag: Int, reqId: Int, payload: ByteArray): ByteArray {
        val length = 5 + payload.size // tag(1) + reqId(4) + payload
        val msg = ByteArray(4 + length)
        writeU32LE(msg, 0, length)
        msg[4] = tag.toByte()
        writeU32LE(msg, 5, reqId)
        System.arraycopy(payload, 0, msg, 9, payload.size)
        return msg
    }

    private fun readU32LE(buf: ByteArray, offset: Int): Int {
        return (buf[offset].toInt() and 0xFF) or
               ((buf[offset + 1].toInt() and 0xFF) shl 8) or
               ((buf[offset + 2].toInt() and 0xFF) shl 16) or
               ((buf[offset + 3].toInt() and 0xFF) shl 24)
    }

    private fun writeU32LE(buf: ByteArray, offset: Int, value: Int) {
        buf[offset] = (value and 0xFF).toByte()
        buf[offset + 1] = ((value shr 8) and 0xFF).toByte()
        buf[offset + 2] = ((value shr 16) and 0xFF).toByte()
        buf[offset + 3] = ((value shr 24) and 0xFF).toByte()
    }

    // ---- Native methods ----
    private external fun nativeCreateSocket(path: String): Int
    private external fun nativeAccept(listenFd: Int): Int
    private external fun nativeReadExact(fd: Int, buf: ByteArray, offset: Int, length: Int): Int
    private external fun nativeWriteExact(fd: Int, buf: ByteArray, offset: Int, length: Int): Int
    private external fun nativeClose(fd: Int)
}
