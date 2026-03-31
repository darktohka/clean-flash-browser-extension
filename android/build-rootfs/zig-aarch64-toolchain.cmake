set(CMAKE_SYSTEM_NAME Android)
set(CMAKE_SYSTEM_VERSION 26)
set(CMAKE_ANDROID_ARCH_ABI arm64-v8a)
set(CMAKE_ANDROID_NDK /opt/android-sdk/ndk/26.1.10909125)
set(CMAKE_ANDROID_STL_TYPE none)

# PRoot needs these defines
add_compile_definitions(_GNU_SOURCE _LARGEFILE64_SOURCE)
