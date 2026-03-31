/**
 * flash_audio.c - AAudio playback bridge for Flash audio output.
 *
 * Provides JNI methods for creating, writing to, starting, stopping,
 * and closing AAudio streams.
 */

#include <jni.h>
#include <android/log.h>
#include <aaudio/AAudio.h>
#include <string.h>
#include <stdlib.h>

#define TAG "FlashAudio"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, TAG, __VA_ARGS__)

/**
 * Create an AAudio output stream.
 * Returns the stream pointer as a jlong, or 0 on failure.
 */
JNIEXPORT jlong JNICALL
Java_com_flashplayer_android_audio_AudioOutputBridge_nativeCreateStream(
        JNIEnv *env, jobject thiz, jint sampleRate, jint channelCount,
        jint framesPerBuffer) {
    AAudioStreamBuilder *builder;
    aaudio_result_t result = AAudio_createStreamBuilder(&builder);
    if (result != AAUDIO_OK) {
        LOGE("AAudio_createStreamBuilder failed: %d", result);
        return 0;
    }

    AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_OUTPUT);
    AAudioStreamBuilder_setPerformanceMode(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
    AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);
    AAudioStreamBuilder_setChannelCount(builder, channelCount);
    AAudioStreamBuilder_setSampleRate(builder, sampleRate);
    AAudioStreamBuilder_setSharingMode(builder, AAUDIO_SHARING_MODE_SHARED);

    AAudioStream *stream;
    result = AAudioStreamBuilder_openStream(builder, &stream);
    AAudioStreamBuilder_delete(builder);

    if (result != AAUDIO_OK) {
        LOGE("AAudioStreamBuilder_openStream failed: %d", result);
        return 0;
    }

    AAudioStream_setBufferSizeInFrames(stream, framesPerBuffer * 2);

    LOGI("Created AAudio stream: rate=%d ch=%d frames=%d",
         sampleRate, channelCount, framesPerBuffer);
    return (jlong)stream;
}

/**
 * Write PCM samples to the AAudio stream.
 * Returns the number of frames written, or a negative error code.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_audio_AudioOutputBridge_nativeWriteSamples(
        JNIEnv *env, jobject thiz, jlong streamPtr, jbyteArray jsamples,
        jint numFrames) {
    AAudioStream *stream = (AAudioStream *)streamPtr;
    if (!stream) return -1;

    jbyte *samples = (*env)->GetByteArrayElements(env, jsamples, NULL);
    if (!samples) return -1;

    aaudio_result_t written = AAudioStream_write(
            stream, samples, numFrames, 100000000LL /* 100ms timeout */);

    (*env)->ReleaseByteArrayElements(env, jsamples, samples, JNI_ABORT);

    if (written < 0) {
        LOGE("AAudioStream_write failed: %d", written);
    }
    return (jint)written;
}

/**
 * Start the AAudio stream.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_audio_AudioOutputBridge_nativeStartStream(
        JNIEnv *env, jobject thiz, jlong streamPtr) {
    AAudioStream *stream = (AAudioStream *)streamPtr;
    if (!stream) return -1;
    aaudio_result_t result = AAudioStream_requestStart(stream);
    LOGI("AAudioStream_requestStart: %d", result);
    return (jint)result;
}

/**
 * Stop the AAudio stream.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_audio_AudioOutputBridge_nativeStopStream(
        JNIEnv *env, jobject thiz, jlong streamPtr) {
    AAudioStream *stream = (AAudioStream *)streamPtr;
    if (!stream) return -1;
    aaudio_result_t result = AAudioStream_requestStop(stream);
    LOGI("AAudioStream_requestStop: %d", result);
    return (jint)result;
}

/**
 * Close and release the AAudio stream.
 */
JNIEXPORT void JNICALL
Java_com_flashplayer_android_audio_AudioOutputBridge_nativeCloseStream(
        JNIEnv *env, jobject thiz, jlong streamPtr) {
    AAudioStream *stream = (AAudioStream *)streamPtr;
    if (stream) {
        AAudioStream_close(stream);
        LOGI("AAudioStream closed");
    }
}
