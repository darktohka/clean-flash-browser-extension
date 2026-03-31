/**
 * shared_memory.c - memfd + mmap helpers for shared memory IPC.
 *
 * Provides JNI methods for creating anonymous shared memory segments
 * using memfd_create, and mapping them for read/write access.
 */

#include <jni.h>
#include <android/log.h>
#include <sys/mman.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
#include <linux/memfd.h>

#define TAG "FlashSHM"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, TAG, __VA_ARGS__)

/* memfd_create may not be in older NDK headers */
#ifndef __NR_memfd_create
#if defined(__aarch64__)
#define __NR_memfd_create 279
#elif defined(__arm__)
#define __NR_memfd_create 385
#elif defined(__x86_64__)
#define __NR_memfd_create 319
#else
#error "Unsupported architecture for memfd_create"
#endif
#endif

static int my_memfd_create(const char *name, unsigned int flags) {
    return (int)syscall(__NR_memfd_create, name, flags);
}

/**
 * Create a memfd (anonymous shared memory file descriptor).
 * Returns the fd, or -1 on failure.
 */
JNIEXPORT jint JNICALL
Java_com_flashplayer_android_ipc_SharedMemory_nativeCreate(
        JNIEnv *env, jclass clazz, jstring jname, jint size) {
    const char *name = (*env)->GetStringUTFChars(env, jname, NULL);
    if (!name) return -1;

    int fd = my_memfd_create(name, MFD_CLOEXEC);
    (*env)->ReleaseStringUTFChars(env, jname, name);

    if (fd < 0) {
        LOGE("memfd_create failed: %s", strerror(errno));
        return -1;
    }

    if (ftruncate(fd, size) < 0) {
        LOGE("ftruncate failed: %s", strerror(errno));
        close(fd);
        return -1;
    }

    LOGI("Created memfd: fd=%d size=%d", fd, size);
    return fd;
}

/**
 * Map a memfd into the process address space.
 * Returns the address as a direct ByteBuffer, or null on failure.
 */
JNIEXPORT jobject JNICALL
Java_com_flashplayer_android_ipc_SharedMemory_nativeMap(
        JNIEnv *env, jclass clazz, jint fd, jint size) {
    void *addr = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (addr == MAP_FAILED) {
        LOGE("mmap failed: %s", strerror(errno));
        return NULL;
    }

    return (*env)->NewDirectByteBuffer(env, addr, size);
}

/**
 * Unmap a previously mapped memory region.
 */
JNIEXPORT void JNICALL
Java_com_flashplayer_android_ipc_SharedMemory_nativeUnmap(
        JNIEnv *env, jclass clazz, jobject buffer, jint size) {
    void *addr = (*env)->GetDirectBufferAddress(env, buffer);
    if (addr) {
        munmap(addr, size);
    }
}
