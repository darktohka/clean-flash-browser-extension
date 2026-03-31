package org.cleanflash.android.ipc

import java.nio.ByteBuffer

/**
 * JNI helper for shared memory operations (memfd_create + mmap).
 */
object SharedMemory {

    init {
        System.loadLibrary("flashplayer")
    }

    /**
     * Create a memfd (anonymous shared memory).
     * Returns the file descriptor, or -1 on failure.
     */
    external fun nativeCreate(name: String, size: Int): Int

    /**
     * Map a memfd into the process address space.
     * Returns a direct ByteBuffer, or null on failure.
     */
    external fun nativeMap(fd: Int, size: Int): ByteBuffer?

    /**
     * Unmap a previously mapped memory region.
     */
    external fun nativeUnmap(buffer: ByteBuffer, size: Int)
}
