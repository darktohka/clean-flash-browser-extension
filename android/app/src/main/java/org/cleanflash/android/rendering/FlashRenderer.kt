package org.cleanflash.android.rendering

import android.opengl.GLES20
import android.opengl.GLES30
import android.opengl.GLSurfaceView
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10

/**
 * OpenGL ES 3.0 renderer for Flash frame display.
 *
 * Receives BGRA pixel data from the flash-host — either via byte-array payload
 * (socket fallback) or a shared-memory mmap'd ByteBuffer (zero-copy fast path) —
 * and uploads dirty regions as texture sub-images.
 * Uses a fragment shader to swizzle BGRA → RGBA.
 */
class FlashRenderer : GLSurfaceView.Renderer {

    private var textureId: Int = 0
    private var programId: Int = 0
    private var textureWidth: Int = 0
    private var textureHeight: Int = 0
    private var surfaceWidth: Int = 0
    private var surfaceHeight: Int = 0

    // Effective viewport after aspect-ratio letterboxing
    var viewportX: Int = 0; private set
    var viewportY: Int = 0; private set
    var viewportW: Int = 0; private set
    var viewportH: Int = 0; private set

    /** Aspect ratio mode: 0 = 4:3, 1 = 16:9, 2 = full (stretch), 3 = SWF native */
    @Volatile var aspectRatioMode: Int = 3
    /** SWF native aspect ratio (set when first frame is detected). */
    @Volatile var swfAspectRatio: Float = 4f / 3f

    // Full frame backing buffer — survives EGL context loss so we can
    // re-upload the entire texture when the GL surface is recreated.
    private var backingBuffer: ByteArray? = null
    private var backingWidth: Int = 0
    private var backingHeight: Int = 0

    // Pending frame update (set from IPC thread, consumed on GL thread)
    @Volatile private var pendingUpdate: FrameUpdate? = null
    // Pending SHM frame update (metadata only — pixels are in sharedBuffer)
    @Volatile private var pendingShmUpdate: ShmFrameUpdate? = null

    // Shared memory buffer (mmap'd from flash-host's framebuffer file)
    @Volatile private var sharedBuffer: ByteBuffer? = null
    private var shmWidth: Int = 0
    private var shmHeight: Int = 0

    private data class FrameUpdate(
        val dx: Int, val dy: Int, val dw: Int, val dh: Int,
        val frameW: Int, val frameH: Int,
        val pixels: ByteArray
    )

    private data class ShmFrameUpdate(
        val dx: Int, val dy: Int, val dw: Int, val dh: Int,
        val frameW: Int, val frameH: Int
    )

    // Vertex data for fullscreen quad
    private val quadVertices: FloatBuffer = ByteBuffer
        .allocateDirect(8 * 4)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()
        .apply {
            put(floatArrayOf(
                -1f, -1f,  // bottom-left
                 1f, -1f,  // bottom-right
                -1f,  1f,  // top-left
                 1f,  1f   // top-right
            ))
            position(0)
        }

    private val texCoords: FloatBuffer = ByteBuffer
        .allocateDirect(8 * 4)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()
        .apply {
            put(floatArrayOf(
                0f, 1f,  // bottom-left (flipped Y for OpenGL)
                1f, 1f,  // bottom-right
                0f, 0f,  // top-left
                1f, 0f   // top-right
            ))
            position(0)
        }

    // BGRA → RGBA swizzle shader
    private val vertexShaderSource = """
        attribute vec4 a_position;
        attribute vec2 a_texCoord;
        varying vec2 v_texCoord;
        void main() {
            gl_Position = a_position;
            v_texCoord = a_texCoord;
        }
    """.trimIndent()

    private val fragmentShaderSource = """
        precision mediump float;
        uniform sampler2D u_texture;
        varying vec2 v_texCoord;
        void main() {
            vec4 color = texture2D(u_texture, v_texCoord);
            gl_FragColor = color.bgra;
        }
    """.trimIndent()

    /**
     * Called from the IPC thread when a new frame arrives with pixel data
     * in the payload (socket fallback path).
     */
    fun updateFrame(dx: Int, dy: Int, dw: Int, dh: Int,
                    frameW: Int, frameH: Int, pixels: ByteArray) {
        pendingUpdate = FrameUpdate(dx, dy, dw, dh, frameW, frameH, pixels)
    }

    /**
     * Called from the IPC thread when a shared-memory frame notification
     * arrives.  The pixels are already in the mmap'd [sharedBuffer].
     */
    fun updateFrameFromShm(dx: Int, dy: Int, dw: Int, dh: Int,
                           frameW: Int, frameH: Int) {
        pendingShmUpdate = ShmFrameUpdate(dx, dy, dw, dh, frameW, frameH)
    }

    /**
     * Store a reference to the mmap'd shared framebuffer.
     */
    fun setSharedBuffer(buffer: ByteBuffer, width: Int, height: Int) {
        sharedBuffer = buffer
        shmWidth = width
        shmHeight = height
    }

    override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
        GLES20.glClearColor(0f, 0f, 0f, 1f)

        // Reset texture dimensions so the next frame update reallocates via
        // glTexImage2D (the old texture is gone after EGL context loss).
        textureWidth = 0
        textureHeight = 0

        // Create shader program
        programId = createProgram(vertexShaderSource, fragmentShaderSource)

        // Create texture
        val textures = IntArray(1)
        GLES20.glGenTextures(1, textures, 0)
        textureId = textures[0]

        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)

        // If we have a backing buffer from before the context loss, schedule
        // a full re-upload so the first frame drawn is not black.
        val bb = backingBuffer
        if (bb != null && backingWidth > 0 && backingHeight > 0) {
            pendingUpdate = FrameUpdate(
                0, 0, backingWidth, backingHeight,
                backingWidth, backingHeight, bb
            )
        }
    }

    override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
        surfaceWidth = width
        surfaceHeight = height
        updateViewport()
    }

    /** Recalculate the GL viewport based on the current aspect ratio mode. */
    fun updateViewport() {
        val w = surfaceWidth
        val h = surfaceHeight
        if (w == 0 || h == 0) return

        val targetRatio = when (aspectRatioMode) {
            0 -> 4f / 3f
            1 -> 16f / 9f
            3 -> swfAspectRatio
            else -> w.toFloat() / h.toFloat() // full — match surface
        }

        val surfaceRatio = w.toFloat() / h.toFloat()
        if (surfaceRatio > targetRatio) {
            // Wider than target → pillarbox (bars on sides)
            viewportH = h
            viewportW = (h * targetRatio).toInt()
            viewportX = (w - viewportW) / 2
            viewportY = 0
        } else {
            // Taller than target → letterbox (bars on top/bottom)
            viewportW = w
            viewportH = (w / targetRatio).toInt()
            viewportX = 0
            viewportY = (h - viewportH) / 2
        }
    }

    override fun onDrawFrame(gl: GL10?) {
        // Clear full surface (gives black letterbox bars)
        GLES20.glViewport(0, 0, surfaceWidth, surfaceHeight)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)

        // Check for pending SHM frame update (preferred)
        val shmUpdate = pendingShmUpdate
        pendingShmUpdate = null

        // Check for pending fallback frame update
        val update = pendingUpdate
        pendingUpdate = null

        if (shmUpdate != null) {
            applyShmFrameUpdate(shmUpdate)
        } else if (update != null) {
            applyFrameUpdate(update)
        }

        if (textureWidth == 0 || textureHeight == 0) return

        // Apply aspect-ratio viewport
        updateViewport()
        GLES20.glViewport(viewportX, viewportY, viewportW, viewportH)

        // Draw the textured quad
        GLES20.glUseProgram(programId)

        val positionHandle = GLES20.glGetAttribLocation(programId, "a_position")
        val texCoordHandle = GLES20.glGetAttribLocation(programId, "a_texCoord")
        val textureHandle = GLES20.glGetUniformLocation(programId, "u_texture")

        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)
        GLES20.glUniform1i(textureHandle, 0)

        GLES20.glEnableVertexAttribArray(positionHandle)
        GLES20.glVertexAttribPointer(positionHandle, 2, GLES20.GL_FLOAT, false, 0, quadVertices)

        GLES20.glEnableVertexAttribArray(texCoordHandle)
        GLES20.glVertexAttribPointer(texCoordHandle, 2, GLES20.GL_FLOAT, false, 0, texCoords)

        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)

        GLES20.glDisableVertexAttribArray(positionHandle)
        GLES20.glDisableVertexAttribArray(texCoordHandle)
    }

    private fun applyFrameUpdate(update: FrameUpdate) {
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)

        // Reallocate texture if frame size changed
        if (update.frameW != textureWidth || update.frameH != textureHeight) {
            textureWidth = update.frameW
            textureHeight = update.frameH

            // Allocate empty texture at new size
            GLES20.glTexImage2D(
                GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA,
                textureWidth, textureHeight, 0,
                GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, null
            )

            // Resize backing buffer
            backingWidth = textureWidth
            backingHeight = textureHeight
            backingBuffer = ByteArray(textureWidth * textureHeight * 4)
        }

        // Upload dirty region as sub-image
        val pixelBuffer = ByteBuffer.allocateDirect(update.pixels.size)
            .order(ByteOrder.nativeOrder())
        pixelBuffer.put(update.pixels)
        pixelBuffer.position(0)

        GLES20.glTexSubImage2D(
            GLES20.GL_TEXTURE_2D, 0,
            update.dx, update.dy, update.dw, update.dh,
            GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, pixelBuffer
        )

        // Update backing buffer so we can restore after context loss
        val bb = backingBuffer
        if (bb != null && backingWidth == update.frameW && backingHeight == update.frameH) {
            val stride = update.frameW * 4
            for (row in 0 until update.dh) {
                val srcOff = row * update.dw * 4
                val dstOff = (update.dy + row) * stride + update.dx * 4
                val len = update.dw * 4
                if (srcOff + len <= update.pixels.size && dstOff + len <= bb.size) {
                    System.arraycopy(update.pixels, srcOff, bb, dstOff, len)
                }
            }
        }
    }

    /**
     * Upload a dirty region from the mmap'd shared buffer using
     * GL_UNPACK_ROW_LENGTH (GLES 3.0) to read strided pixel data directly.
     */
    private fun applyShmFrameUpdate(update: ShmFrameUpdate) {
        val buf = sharedBuffer ?: return

        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)

        if (update.frameW != textureWidth || update.frameH != textureHeight) {
            textureWidth = update.frameW
            textureHeight = update.frameH
            GLES20.glTexImage2D(
                GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA,
                textureWidth, textureHeight, 0,
                GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, null
            )
            backingWidth = textureWidth
            backingHeight = textureHeight
            backingBuffer = ByteArray(textureWidth * textureHeight * 4)
        }

        // Position the buffer at the start of the dirty region and set
        // GL_UNPACK_ROW_LENGTH so GL reads rows with the full-frame stride.
        val offset = (update.dy * update.frameW + update.dx) * 4
        buf.position(offset)

        GLES30.glPixelStorei(GLES30.GL_UNPACK_ROW_LENGTH, update.frameW)
        GLES20.glTexSubImage2D(
            GLES20.GL_TEXTURE_2D, 0,
            update.dx, update.dy, update.dw, update.dh,
            GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, buf
        )
        GLES30.glPixelStorei(GLES30.GL_UNPACK_ROW_LENGTH, 0)

        // Update backing buffer from the mmap'd data
        val bb = backingBuffer
        if (bb != null) {
            val rowBytes = update.dw * 4
            for (row in 0 until update.dh) {
                val off = ((update.dy + row) * update.frameW + update.dx) * 4
                if (off + rowBytes <= buf.capacity() && off + rowBytes <= bb.size) {
                    buf.position(off)
                    buf.get(bb, off, rowBytes)
                }
            }
        }
    }

    private fun createProgram(vertexSource: String, fragmentSource: String): Int {
        val vertexShader = loadShader(GLES20.GL_VERTEX_SHADER, vertexSource)
        val fragmentShader = loadShader(GLES20.GL_FRAGMENT_SHADER, fragmentSource)

        val program = GLES20.glCreateProgram()
        GLES20.glAttachShader(program, vertexShader)
        GLES20.glAttachShader(program, fragmentShader)
        GLES20.glLinkProgram(program)

        val linkStatus = IntArray(1)
        GLES20.glGetProgramiv(program, GLES20.GL_LINK_STATUS, linkStatus, 0)
        if (linkStatus[0] == 0) {
            val log = GLES20.glGetProgramInfoLog(program)
            GLES20.glDeleteProgram(program)
            throw RuntimeException("Program link failed: $log")
        }

        return program
    }

    private fun loadShader(type: Int, source: String): Int {
        val shader = GLES20.glCreateShader(type)
        GLES20.glShaderSource(shader, source)
        GLES20.glCompileShader(shader)

        val compileStatus = IntArray(1)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, compileStatus, 0)
        if (compileStatus[0] == 0) {
            val log = GLES20.glGetShaderInfoLog(shader)
            GLES20.glDeleteShader(shader)
            throw RuntimeException("Shader compile failed: $log")
        }

        return shader
    }
}
