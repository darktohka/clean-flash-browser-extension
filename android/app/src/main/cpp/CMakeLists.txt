cmake_minimum_required(VERSION 3.22.1)

project("flashplayer" LANGUAGES C)

# IPC epoll - Unix socket server with epoll for efficient multiplexing
add_library(flashplayer SHARED
    ipc_socket.c
    flash_audio.c
    shared_memory.c
)

find_library(log-lib log)
find_library(android-lib android)

target_link_libraries(flashplayer
    ${log-lib}
    ${android-lib}
    aaudio
)

target_compile_options(flashplayer PRIVATE -Wall -Wextra -O2)
