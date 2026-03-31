/**
 * ipc_socket.c - AF_UNIX socket server with epoll for flash-host IPC.
 *
 * Provides JNI methods for creating, accepting, reading/writing on Unix
 * domain stream sockets using epoll for efficient I/O multiplexing.
 */

#include <jni.h>
#include <android/log.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/epoll.h>
#include <sys/eventfd.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <stdlib.h>

#define TAG "FlashIPC"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, TAG, __VA_ARGS__)

/**
 * Create a listening AF_UNIX stream socket at the given path.
 * Returns the socket fd, or -1 on error.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_ipc_IpcServer_nativeCreateSocket(
        JNIEnv *env, jobject thiz, jstring jpath) {
    const char *path = (*env)->GetStringUTFChars(env, jpath, NULL);
    if (!path) return -1;

    // Remove existing socket file
    unlink(path);

    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) {
        LOGE("socket() failed: %s", strerror(errno));
        (*env)->ReleaseStringUTFChars(env, jpath, path);
        return -1;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path, sizeof(addr.sun_path) - 1);

    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        LOGE("bind(%s) failed: %s", path, strerror(errno));
        close(fd);
        (*env)->ReleaseStringUTFChars(env, jpath, path);
        return -1;
    }

    if (listen(fd, 1) < 0) {
        LOGE("listen() failed: %s", strerror(errno));
        close(fd);
        (*env)->ReleaseStringUTFChars(env, jpath, path);
        return -1;
    }

    LOGI("Listening on %s (fd=%d)", path, fd);
    (*env)->ReleaseStringUTFChars(env, jpath, path);
    return fd;
}

/**
 * Accept a connection on the listening socket.
 * Blocks until a client connects. Returns the client fd.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_ipc_IpcServer_nativeAccept(
        JNIEnv *env, jobject thiz, jint listenFd) {
    int client = accept(listenFd, NULL, NULL);
    if (client < 0) {
        LOGE("accept() failed: %s", strerror(errno));
        return -1;
    }
    LOGI("Accepted client fd=%d on listen fd=%d", client, listenFd);
    return client;
}

/**
 * Read exactly `length` bytes from the socket into the given byte array.
 * Returns 0 on success, -1 on error/EOF.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_ipc_IpcServer_nativeReadExact(
        JNIEnv *env, jobject thiz, jint fd, jbyteArray jbuf, jint offset, jint length) {
    jbyte *buf = (*env)->GetByteArrayElements(env, jbuf, NULL);
    if (!buf) return -1;

    int total = 0;
    while (total < length) {
        ssize_t n = read(fd, buf + offset + total, length - total);
        if (n <= 0) {
            (*env)->ReleaseByteArrayElements(env, jbuf, buf, 0);
            return -1;
        }
        total += (int)n;
    }

    (*env)->ReleaseByteArrayElements(env, jbuf, buf, 0);
    return 0;
}

/**
 * Write exactly `length` bytes from the byte array to the socket.
 * Returns 0 on success, -1 on error.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_ipc_IpcServer_nativeWriteExact(
        JNIEnv *env, jobject thiz, jint fd, jbyteArray jbuf, jint offset, jint length) {
    jbyte *buf = (*env)->GetByteArrayElements(env, jbuf, NULL);
    if (!buf) return -1;

    int total = 0;
    while (total < length) {
        ssize_t n = write(fd, buf + offset + total, length - total);
        if (n <= 0) {
            (*env)->ReleaseByteArrayElements(env, jbuf, buf, JNI_ABORT);
            return -1;
        }
        total += (int)n;
    }

    (*env)->ReleaseByteArrayElements(env, jbuf, buf, JNI_ABORT);
    return 0;
}

/**
 * Close a file descriptor.
 */
JNIEXPORT void JNICALL
Java_com_flashplayer_android_ipc_IpcServer_nativeClose(
        JNIEnv *env, jobject thiz, jint fd) {
    if (fd >= 0) close(fd);
}
