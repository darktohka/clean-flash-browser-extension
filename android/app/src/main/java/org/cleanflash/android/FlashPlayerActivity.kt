package org.cleanflash.android

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import android.widget.LinearLayout
import android.widget.TextView
import org.cleanflash.android.ipc.IpcServer
import org.cleanflash.android.ipc.MessageCodec
import org.cleanflash.android.ipc.SharedMemory
import org.cleanflash.android.input.TouchHandler
import org.cleanflash.android.rendering.FlashRenderer
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.channels.FileChannel
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10

/**
 * Fullscreen activity that plays a SWF file.
 *
 * Manages the host process lifecycle, IPC communication, frame rendering,
 * and input forwarding.
 */
class FlashPlayerActivity : Activity() {

    companion object {
        const val EXTRA_SWF_SOURCE = "swf_source"
        const val EXTRA_IS_LOCAL_FILE = "is_local_file"
    }

    private lateinit var glSurfaceView: GLSurfaceView
    private lateinit var loadingOverlay: View
    private lateinit var loadingText: TextView
    private lateinit var controlBar: LinearLayout
    private lateinit var touchHandler: TouchHandler

    private var ipcServer: IpcServer? = null
    private var flashRenderer: FlashRenderer? = null
    private var containerManager: ContainerManager? = null
    private var hostProcess: Process? = null

    private var swfSource: String = ""
    private var isLocalFile: Boolean = false
    private var flashWidth: Int = 800
    private var flashHeight: Int = 600
    private var isControlBarVisible = false

    // Shared memory framebuffer (mmap'd file shared with flash-host)
    private var frameShmBuffer: ByteBuffer? = null
    private var frameShmWidth: Int = 0
    private var frameShmHeight: Int = 0

    // Settings — aspect ratio and render resolution
    /** 0 = 4:3, 1 = 16:9, 2 = full screen (stretch to native), 3 = SWF native */
    private var aspectRatioMode: Int = 3  // default: auto-detect from SWF
    /** Render resolution multiplier: 0.25, 0.5, 0.75, or 1.0 */
    private var renderScale: Float = 1.0f
    /** Detected SWF stage dimensions (0 = not yet known). */
    private var swfStageWidth: Int = 0
    private var swfStageHeight: Int = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_player)

        // Enter immersive mode
        enterImmersiveMode()

        // Keep screen on
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        // Get views
        glSurfaceView = findViewById(R.id.flash_surface)
        loadingOverlay = findViewById(R.id.loading_overlay)
        loadingText = findViewById(R.id.loading_text)
        controlBar = findViewById(R.id.control_bar)

        // Read intent extras — also accept intent data URI as fallback
        swfSource = intent.getStringExtra(EXTRA_SWF_SOURCE)
            ?: intent.data?.toString()
            ?: ""
        isLocalFile = intent.getBooleanExtra(EXTRA_IS_LOCAL_FILE, false)

        if (swfSource.isEmpty()) {
            loadingText.text = "No SWF file specified"
            return
        }

        // Set up touch handler
        touchHandler = TouchHandler { tag, payload ->
            ipcServer?.sendToHost(tag, payload)
        }

        // Set up touch overlay
        val touchOverlay = findViewById<View>(R.id.touch_overlay)
        touchOverlay.setOnTouchListener { _, event ->
            val renderer = flashRenderer
            if (renderer != null) {
                // Map touch coords through the aspect-ratio viewport
                touchHandler.onTouchEvent(
                    event, flashWidth, flashHeight,
                    renderer.viewportW.coerceAtLeast(1),
                    renderer.viewportH.coerceAtLeast(1),
                    renderer.viewportX, renderer.viewportY
                )
            }
            true
        }

        // Triple-tap to toggle control bar
        touchOverlay.setOnClickListener {
            toggleControlBar()
        }

        // Control bar buttons
        findViewById<View>(R.id.btn_keyboard).setOnClickListener {
            showSoftKeyboard()
        }

        findViewById<View>(R.id.btn_menu).setOnClickListener { anchor ->
            showSettingsMenu(anchor)
        }

        // Always-visible settings button
        findViewById<View>(R.id.btn_settings).setOnClickListener { anchor ->
            showSettingsMenu(anchor)
        }

        // Set up GL surface
        flashRenderer = FlashRenderer()
        glSurfaceView.setEGLContextClientVersion(3)
        glSurfaceView.setRenderer(flashRenderer)
        glSurfaceView.renderMode = GLSurfaceView.RENDERMODE_WHEN_DIRTY

        // Enable keyboard focus
        glSurfaceView.isFocusable = true
        glSurfaceView.isFocusableInTouchMode = true

        // Start Flash host
        loadingText.text = "Initializing container..."
        Thread { startFlashHost() }.start()
    }

    private fun startFlashHost() {
        val container = ContainerManager(this)
        containerManager = container

        // Initialize container (rootfs + Box64 + host binary)
        container.initialize { stage, detail ->
            runOnUiThread {
                loadingText.text = when (stage) {
                    "rootfs" -> "Setting up system files...\n$detail"
                    "box64" -> "Installing Box64...\n$detail"
                    "host" -> "Installing flash-host..."
                    else -> detail
                }
            }
        }

        // Clean temp files from previous sessions
        container.cleanTempFiles()

        // Prepare SWF source
        val swfUrl = if (isLocalFile) {
            val tempPath = container.copyFileToChroot(Uri.parse(swfSource))
            "file://$tempPath"
        } else {
            swfSource
        }

        // Create IPC server
        runOnUiThread { loadingText.text = "Starting IPC server..." }
        val socketPath = container.getSocketPath("control.sock")
        val server = IpcServer(socketPath)
        ipcServer = server

        // Set up message handler
        server.setMessageHandler { tag, reqId, payload ->
            handleHostMessage(tag, reqId, payload)
        }

        // Start listening
        server.start()

        // Launch host process
        runOnUiThread { loadingText.text = "Launching Flash host..." }
        val surfaceWidth = glSurfaceView.width.coerceAtLeast(800)
        val surfaceHeight = glSurfaceView.height.coerceAtLeast(600)
        val (renderW, renderH) = computeRenderSize(surfaceWidth, surfaceHeight)

        hostProcess = container.launchHost(
            socketPath = socketPath,
            swfUrl = swfUrl,
            width = renderW,
            height = renderH,
            enableLogs = true,
            onTermination = { exitCode ->
                runOnUiThread {
                    if (exitCode != 0) {
                        val log = container.getHostLog()?.takeLast(500) ?: "No log available"
                        loadingText.text = "Flash host exited (code $exitCode)\n$log"
                        loadingOverlay.visibility = View.VISIBLE
                    }
                }
            }
        )

        if (hostProcess == null) {
            runOnUiThread {
                loadingText.text = "Failed to launch Flash host"
            }
            return
        }

        // Wait for host to connect
        runOnUiThread { loadingText.text = "Waiting for host connection..." }
        server.waitForConnection(30_000)

        // Send open command
        val openPayload = MessageCodec.buildOpenCommand(swfUrl, renderW, renderH)
        server.sendToHost(MessageCodec.TAG_OPEN, openPayload)

        runOnUiThread { loadingText.text = "Loading Flash content..." }
    }

    /**
     * Handle messages from the flash-host process.
     */
    private fun handleHostMessage(tag: Int, reqId: Int, payload: ByteArray) {
        when (tag) {
            MessageCodec.TAG_FRAME_READY -> {
                handleFrameReady(payload)
            }
            MessageCodec.TAG_FRAME_INIT -> {
                handleFrameInit(payload)
            }
            MessageCodec.TAG_STATE_CHANGE -> {
                handleStateChange(payload)
            }
            MessageCodec.TAG_CURSOR_CHANGE -> {
                // Could update cursor icon on Android (limited support)
            }
            MessageCodec.TAG_NAVIGATE -> {
                handleNavigate(payload)
            }
            MessageCodec.TAG_VERSION -> {
                // Host version received
            }
            MessageCodec.TAG_AUDIO_INIT -> {
                handleAudioInit(payload)
            }
            MessageCodec.TAG_AUDIO_START -> {
                handleAudioStart(payload)
            }
            MessageCodec.TAG_AUDIO_STOP -> {
                handleAudioStop(payload)
            }
            MessageCodec.TAG_AUDIO_CLOSE -> {
                handleAudioClose(payload)
            }
            MessageCodec.TAG_AUDIO_SAMPLES -> {
                handleAudioSamples(payload)
            }
            MessageCodec.TAG_HTTP_REQUEST -> {
                handleHttpRequest(reqId, payload)
            }
            MessageCodec.TAG_CLIPBOARD_READ -> {
                handleClipboardRead(reqId, payload)
            }
            MessageCodec.TAG_CLIPBOARD_WRITE -> {
                handleClipboardWrite(reqId, payload)
            }
            MessageCodec.TAG_COOKIE_GET -> {
                handleCookieGet(reqId, payload)
            }
            MessageCodec.TAG_COOKIE_SET -> {
                handleCookieSet(payload)
            }
            MessageCodec.TAG_CONTEXT_MENU_SHOW -> {
                handleContextMenuShow(reqId, payload)
            }
            MessageCodec.TAG_DIALOG_SHOW -> {
                handleDialogShow(reqId, payload)
            }
            MessageCodec.TAG_FILE_CHOOSER_SHOW -> {
                handleFileChooserShow(reqId, payload)
            }
            MessageCodec.TAG_FULLSCREEN_QUERY -> {
                handleFullscreenQuery(reqId)
            }
        }
    }

    private fun handleFrameInit(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val w = reader.readU32()
        val h = reader.readU32()
        val guestPath = reader.readString()

        // Translate guest path to real filesystem path via the rootfs dir.
        val rootfs = containerManager?.rootfsDir ?: return
        val realPath = File(rootfs, guestPath.removePrefix("/"))
        if (!realPath.exists()) {
            android.util.Log.e("FlashPlayer", "SHM file not found: $realPath")
            return
        }

        try {
            val raf = RandomAccessFile(realPath, "r")
            val channel = raf.channel
            val buf = channel.map(FileChannel.MapMode.READ_ONLY, 0, (w * h * 4).toLong())
            buf.order(ByteOrder.nativeOrder())
            frameShmBuffer = buf
            frameShmWidth = w
            frameShmHeight = h
            flashRenderer?.setSharedBuffer(buf, w, h)
            android.util.Log.i("FlashPlayer", "SHM framebuffer mapped: ${w}x${h} from $realPath")
            // Channel/raf can be closed; the mapping stays valid.
            channel.close()
            raf.close()
        } catch (e: Exception) {
            android.util.Log.e("FlashPlayer", "Failed to mmap SHM file", e)
        }
    }

    private fun handleFrameReady(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val dx = reader.readU32()
        val dy = reader.readU32()
        val dw = reader.readU32()
        val dh = reader.readU32()
        val frameW = reader.readU32()
        val frameH = reader.readU32()

        flashWidth = frameW
        flashHeight = frameH

        // Auto-detect SWF stage size from the very first frame.
        if (swfStageWidth == 0 && swfStageHeight == 0) {
            swfStageWidth = frameW
            swfStageHeight = frameH
            android.util.Log.i("FlashPlayer", "Detected SWF stage size: ${frameW}x${frameH}")
            // Update renderer aspect ratio from actual stage
            if (aspectRatioMode == 3) {
                flashRenderer?.let { renderer ->
                    renderer.aspectRatioMode = 3
                    renderer.swfAspectRatio = frameW.toFloat() / frameH.coerceAtLeast(1).toFloat()
                }
            }
        }

        val shm = frameShmBuffer
        if (shm != null && frameShmWidth == frameW && frameShmHeight == frameH) {
            // Fast path: pixels are already in the mmap'd buffer.
            flashRenderer?.updateFrameFromShm(dx, dy, dw, dh, frameW, frameH)
        } else {
            // Fallback: read pixel data from the IPC payload.
            val pixels = reader.readRemaining()
            flashRenderer?.updateFrame(dx, dy, dw, dh, frameW, frameH, pixels)
        }
        glSurfaceView.requestRender()

        // Hide loading overlay on first frame
        if (loadingOverlay.visibility == View.VISIBLE) {
            runOnUiThread {
                loadingOverlay.visibility = View.GONE
            }
        }
    }

    private fun handleStateChange(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val state = reader.readU8()
        val width = reader.readI32()
        val height = reader.readI32()

        when (state) {
            1 -> { // Running
                flashWidth = width
                flashHeight = height
                runOnUiThread {
                    loadingOverlay.visibility = View.GONE
                }
                // Send view update so that Flash knows we're visible and focused
                val viewPayload = MessageCodec.PayloadWriter()
                viewPayload.writeU8(1) // visible
                viewPayload.writeU8(1) // focused
                ipcServer?.sendToHost(MessageCodec.TAG_VIEW_UPDATE, viewPayload.finish())

                // Also send a resize with the actual render dimensions
                sendRenderResize()
            }
            3 -> { // Error
                runOnUiThread {
                    loadingText.text = "Error loading Flash content"
                }
            }
        }
    }

    private fun handleNavigate(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val url = reader.readString()
        // Open in browser
        try {
            val intent = android.content.Intent(
                android.content.Intent.ACTION_VIEW,
                Uri.parse(url)
            )
            startActivity(intent)
        } catch (e: Exception) {
            // Ignore if no browser available
        }
    }

    // ---- Audio handlers ----
    private val audioStreams = HashMap<Int, Long>() // streamId -> native stream ptr

    private fun handleAudioInit(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val streamId = reader.readU32()
        val sampleRate = reader.readU32()
        val frameCount = reader.readU32()

        // AAudio only accepts realistic sample rates and frame sizes.
        if (sampleRate !in 8_000..192_000 || frameCount <= 0) {
            return
        }

        val ptr = org.cleanflash.android.audio.AudioOutputBridge.nativeCreateStream(
            sampleRate, 2, frameCount
        )
        if (ptr != 0L) {
            synchronized(audioStreams) { audioStreams[streamId] = ptr }
        }
    }

    private fun handleAudioStart(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val streamId = reader.readU32()
        val ptr = synchronized(audioStreams) { audioStreams[streamId] } ?: return
        org.cleanflash.android.audio.AudioOutputBridge.nativeStartStream(ptr)
    }

    private fun handleAudioStop(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val streamId = reader.readU32()
        val ptr = synchronized(audioStreams) { audioStreams[streamId] } ?: return
        org.cleanflash.android.audio.AudioOutputBridge.nativeStopStream(ptr)
    }

    private fun handleAudioClose(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val streamId = reader.readU32()
        val ptr = synchronized(audioStreams) { audioStreams.remove(streamId) } ?: return
        org.cleanflash.android.audio.AudioOutputBridge.nativeCloseStream(ptr)
    }

    private fun handleAudioSamples(payload: ByteArray) {
        val reader = MessageCodec.PayloadReader(payload)
        val streamId = reader.readU32()
        val pcm = reader.readBytes()
        if (pcm.isEmpty()) return

        val ptr = synchronized(audioStreams) { audioStreams[streamId] } ?: return
        val bytesPerFrame = 2 * 2 // stereo, 16-bit PCM
        val frames = pcm.size / bytesPerFrame
        if (frames <= 0) return

        org.cleanflash.android.audio.AudioOutputBridge.nativeWriteSamples(ptr, pcm, frames)
    }

    // ---- HTTP handler ----
    private fun handleHttpRequest(reqId: Int, payload: ByteArray) {
        Thread {
            val response = org.cleanflash.android.bridges.HttpProxy.executeRequest(payload)
            ipcServer?.sendResponse(MessageCodec.TAG_HTTP_RESPONSE, reqId, response)
        }.start()
    }

    // ---- Clipboard handlers ----
    private fun handleClipboardRead(reqId: Int, payload: ByteArray) {
        runOnUiThread {
            val response = org.cleanflash.android.bridges.ClipboardBridge.readClipboard(this, payload)
            ipcServer?.sendResponse(MessageCodec.TAG_CLIPBOARD_RESPONSE, reqId, response)
        }
    }

    private fun handleClipboardWrite(reqId: Int, payload: ByteArray) {
        runOnUiThread {
            org.cleanflash.android.bridges.ClipboardBridge.writeClipboard(this, payload)
            ipcServer?.sendResponse(MessageCodec.TAG_CLIPBOARD_RESPONSE, reqId, byteArrayOf(1))
        }
    }

    // ---- Cookie handlers ----
    private fun handleCookieGet(reqId: Int, payload: ByteArray) {
        val response = org.cleanflash.android.bridges.CookieStore.getCookies(this, payload)
        ipcServer?.sendResponse(MessageCodec.TAG_COOKIE_RESPONSE, reqId, response)
    }

    private fun handleCookieSet(payload: ByteArray) {
        org.cleanflash.android.bridges.CookieStore.setCookies(this, payload)
    }

    // ---- Context menu handler ----
    private fun handleContextMenuShow(reqId: Int, payload: ByteArray) {
        runOnUiThread {
            org.cleanflash.android.bridges.ContextMenuBridge.showMenu(this, payload) { selectedId ->
                val response = MessageCodec.PayloadWriter()
                response.writeI32(selectedId)
                ipcServer?.sendResponse(MessageCodec.TAG_MENU_RESPONSE, reqId, response.finish())
            }
        }
    }

    // ---- Dialog handler ----
    private fun handleDialogShow(reqId: Int, payload: ByteArray) {
        runOnUiThread {
            org.cleanflash.android.bridges.DialogBridge.showDialog(this, payload) { response ->
                ipcServer?.sendResponse(MessageCodec.TAG_DIALOG_RESPONSE, reqId, response)
            }
        }
    }

    // ---- File chooser handler ----
    private fun handleFileChooserShow(reqId: Int, payload: ByteArray) {
        // File chooser requires ActivityResult, simplified for now
        val response = MessageCodec.PayloadWriter()
        response.writeU32(0) // 0 files selected
        ipcServer?.sendResponse(MessageCodec.TAG_FILE_CHOOSER_RESPONSE, reqId, response.finish())
    }

    // ---- Fullscreen handler ----
    private fun handleFullscreenQuery(reqId: Int) {
        val display = windowManager.defaultDisplay
        val size = android.graphics.Point()
        display.getRealSize(size)

        val response = MessageCodec.PayloadWriter()
        response.writeI32(size.x)
        response.writeI32(size.y)
        ipcServer?.sendResponse(MessageCodec.TAG_FULLSCREEN_QUERY + 0x40, reqId, response.finish())
    }

    // ---- Input handling ----
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val ipc = ipcServer ?: return super.dispatchKeyEvent(event)

        val tag = when (event.action) {
            KeyEvent.ACTION_DOWN -> MessageCodec.TAG_KEY_DOWN
            KeyEvent.ACTION_UP -> MessageCodec.TAG_KEY_UP
            else -> return super.dispatchKeyEvent(event)
        }

        val pw = MessageCodec.PayloadWriter()
        pw.writeU32(event.keyCode)
        pw.writeU32(translateModifiers(event))
        pw.writeString(event.unicodeChar.toChar().toString())
        pw.writeString(KeyEvent.keyCodeToString(event.keyCode))
        ipc.sendToHost(tag, pw.finish())
        return true
    }

    private fun translateModifiers(event: KeyEvent): Int {
        var mods = 0
        if (event.isShiftPressed) mods = mods or 1
        if (event.isCtrlPressed) mods = mods or 2
        if (event.isAltPressed) mods = mods or 4
        if (event.isMetaPressed) mods = mods or 8
        return mods
    }

    // ---- UI helpers ----
    private fun enterImmersiveMode() {
        window.decorView.systemUiVisibility = (
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
            or View.SYSTEM_UI_FLAG_FULLSCREEN
            or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
            or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
            or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
        )
    }

    private fun toggleControlBar() {
        isControlBarVisible = !isControlBarVisible
        controlBar.visibility = if (isControlBarVisible) View.VISIBLE else View.GONE
    }

    private fun showSoftKeyboard() {
        glSurfaceView.requestFocus()
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.showSoftInput(glSurfaceView, InputMethodManager.SHOW_IMPLICIT)
    }

    private fun hideSoftKeyboard() {
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(glSurfaceView.windowToken, 0)
    }

    /** Compute render dimensions that respect aspect ratio + renderScale. */
    private fun computeRenderSize(surfaceW: Int, surfaceH: Int): Pair<Int, Int> {
        val targetRatio = when (aspectRatioMode) {
            0 -> 4f / 3f
            1 -> 16f / 9f
            3 -> if (swfStageWidth > 0 && swfStageHeight > 0)
                     swfStageWidth.toFloat() / swfStageHeight.toFloat()
                 else 4f / 3f  // fallback before first frame
            else -> surfaceW.toFloat() / surfaceH.toFloat() // mode 2: full
        }

        val surfaceRatio = surfaceW.toFloat() / surfaceH.coerceAtLeast(1).toFloat()
        val (fitW, fitH) = if (surfaceRatio > targetRatio) {
            // pillarbox: height-limited
            ((surfaceH * targetRatio).toInt() to surfaceH)
        } else {
            // letterbox: width-limited
            (surfaceW to (surfaceW / targetRatio).toInt())
        }

        val rw = (fitW * renderScale).toInt().coerceAtLeast(1)
        val rh = (fitH * renderScale).toInt().coerceAtLeast(1)
        return rw to rh
    }

    /** Send a resize to flash-host using the current renderScale + aspect ratio. */
    private fun sendRenderResize() {
        val sw = glSurfaceView.width.coerceAtLeast(1)
        val sh = glSurfaceView.height.coerceAtLeast(1)
        val (rw, rh) = computeRenderSize(sw, sh)
        val pw = MessageCodec.PayloadWriter()
        pw.writeU32(rw)
        pw.writeU32(rh)
        ipcServer?.sendToHost(MessageCodec.TAG_RESIZE, pw.finish())
    }

    private fun showSettingsMenu(anchor: View) {
        val arLabels = arrayOf("4:3", "16:9", "Full Screen", "SWF Native")
        val rrLabels = arrayOf("25%", "50%", "75%", "100%")
        val rrValues = floatArrayOf(0.25f, 0.5f, 0.75f, 1.0f)

        // Build a flat list of options with section headers
        val items = mutableListOf<String>()
        // Aspect ratio section
        items.add("— Aspect Ratio —")
        val arOffset = items.size
        for (i in arLabels.indices) {
            val check = if (i == aspectRatioMode) "◉ " else "○ "
            items.add("$check${arLabels[i]}")
        }
        // Render resolution section
        items.add("— Render Resolution —")
        val rrOffset = items.size
        for (i in rrLabels.indices) {
            val check = if (renderScale == rrValues[i]) "◉ " else "○ "
            items.add("$check${rrLabels[i]}")
        }
        // Actions section
        items.add("—")
        val kbIndex = items.size
        items.add("⌨ Show Keyboard")

        AlertDialog.Builder(this)
            .setItems(items.toTypedArray()) { _, which ->
                when {
                    which in arOffset until (arOffset + arLabels.size) -> {
                        aspectRatioMode = which - arOffset
                        flashRenderer?.let { r ->
                            r.aspectRatioMode = aspectRatioMode
                            if (aspectRatioMode == 3 && swfStageWidth > 0 && swfStageHeight > 0) {
                                r.swfAspectRatio = swfStageWidth.toFloat() / swfStageHeight.toFloat()
                            }
                        }
                        sendRenderResize()
                        glSurfaceView.requestRender()
                    }
                    which in rrOffset until (rrOffset + rrLabels.size) -> {
                        renderScale = rrValues[which - rrOffset]
                        sendRenderResize()
                    }
                    which == kbIndex -> {
                        showSoftKeyboard()
                    }
                }
            }
            .show()
    }

    // ---- Lifecycle ----
    override fun onResume() {
        super.onResume()
        if (flashRenderer != null) glSurfaceView.onResume()
        enterImmersiveMode()

        // Notify host of focus and request a full repaint.
        // After an EGL context loss the texture is blank, so we need
        // Flash to re-send a complete frame via DidChangeView.
        val pw = MessageCodec.PayloadWriter()
        pw.writeU8(1) // has focus
        ipcServer?.sendToHost(MessageCodec.TAG_FOCUS, pw.finish())

        // Send VIEW_UPDATE to trigger a DidChangeView inside the host,
        // which causes Flash to repaint the entire surface.
        val viewPw = MessageCodec.PayloadWriter()
        viewPw.writeU8(1) // visible
        viewPw.writeU8(1) // focused
        ipcServer?.sendToHost(MessageCodec.TAG_VIEW_UPDATE, viewPw.finish())
    }

    override fun onPause() {
        super.onPause()
        if (flashRenderer != null) glSurfaceView.onPause()

        // Notify host of focus loss
        val pw = MessageCodec.PayloadWriter()
        pw.writeU8(0) // lost focus
        ipcServer?.sendToHost(MessageCodec.TAG_FOCUS, pw.finish())
    }

    override fun onDestroy() {
        super.onDestroy()
        // Send close command
        ipcServer?.sendToHost(MessageCodec.TAG_CLOSE, ByteArray(0))

        // Clean up
        containerManager?.stopHost()
        ipcServer?.stop()

        // Close audio streams
        synchronized(audioStreams) {
            for ((_, ptr) in audioStreams) {
                org.cleanflash.android.audio.AudioOutputBridge.nativeCloseStream(ptr)
            }
            audioStreams.clear()
        }
    }
}
