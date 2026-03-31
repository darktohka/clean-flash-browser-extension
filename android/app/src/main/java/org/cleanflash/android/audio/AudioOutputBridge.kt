package org.cleanflash.android.audio

/**
 * JNI bridge to AAudio for audio output.
 */
object AudioOutputBridge {

    init {
        System.loadLibrary("flashplayer")
    }

    /**
     * Create an AAudio output stream.
     * Returns the native stream pointer, or 0 on failure.
     */
    external fun nativeCreateStream(sampleRate: Int, channelCount: Int,
                                     framesPerBuffer: Int): Long

    /**
     * Write PCM samples to the stream.
     * Returns number of frames written, or negative on error.
     */
    external fun nativeWriteSamples(streamPtr: Long, samples: ByteArray,
                                     numFrames: Int): Int

    /** Start playback. */
    external fun nativeStartStream(streamPtr: Long): Int

    /** Stop playback. */
    external fun nativeStopStream(streamPtr: Long): Int

    /** Close and release the stream. */
    external fun nativeCloseStream(streamPtr: Long)
}
