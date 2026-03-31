# Build & Setup Guide

## Overview

The Clean Flash Player Android app runs `libpepflashplayer.so` (x86_64 Linux PPAPI plugin) on ARM64
Android devices using PRoot for filesystem isolation and Box64 for x86_64→ARM64 translation.

## Components

1. **Rootfs** — Minimal Ubuntu 24.04 sysroot with x86_64 glibc + aarch64 glibc
2. **Box64** — x86_64→ARM64 dynamic binary translator (from Winlator)
3. **PRoot** — Userspace chroot implementation (built from Winlator sources with zig)
4. **flash-host** — x86_64 static binary that loads the Flash plugin (runs under Box64)
5. **Android app** — Kotlin/JNI app for rendering, audio, input, and networking

## Build Steps

### 1. Build the Rootfs

```bash
cd build-rootfs/
./build.sh                    # Just rootfs
./build.sh --with-box64       # Rootfs + Box64
```

This produces:
- `build-rootfs/output/rootfs.tar.zst` (~15-20 MB)
- `build-rootfs/output/box64-v0.2.8.tar.zst` (optional)

Copy these to the Android assets:
```bash
mkdir -p installer/app/src/main/assets/
cp build-rootfs/output/rootfs.tar.zst installer/app/src/main/assets/
cp build-rootfs/output/box64-v0.2.8.tar.zst installer/app/src/main/assets/
```

### 2. Build PRoot for Android

PRoot must be compiled as `libproot.so` for Android ARM64. You also need `libproot-loader.so`.

The PRoot source lives in `winlator/app/src/main/cpp/proot/`. Cross-compile for
aarch64-linux using zig:

```bash
mkdir -p build-rootfs/proot-build && cd build-rootfs/proot-build
cmake -DCMAKE_TOOLCHAIN_FILE=../zig-aarch64-toolchain.cmake \
      -DCMAKE_C_LINKER_DEPFILE_SUPPORTED=FALSE \
      ../../winlator/app/src/main/cpp/proot
cmake --build .
```

This produces `libproot.so` (executable) and `libproot-loader.so` (shared lib).

Place the binaries in:
```
installer/app/src/main/jniLibs/arm64-v8a/
├── libproot.so           # PRoot binary
└── libproot-loader.so    # PRoot ELF loader stub
```

### 2b. Extract Box64 from Winlator

Box64 is an aarch64 binary packaged in `winlator/installable_components/box64/`.
Extract, patch its interpreter/runpath for our rootfs, and repackage:

```bash
zstd -d winlator/installable_components/box64/box64-0.3.7.tzst -o /tmp/box64.tar
tar xf /tmp/box64.tar -C /tmp ./usr/local/bin/box64
patchelf --set-interpreter /lib/aarch64-linux-gnu/ld-linux-aarch64.so.1 \
         --set-rpath /lib/aarch64-linux-gnu /tmp/usr/local/bin/box64

# Package for assets
mkdir -p /tmp/box64-staging/usr/local/bin
cp /tmp/usr/local/bin/box64 /tmp/box64-staging/usr/local/bin/
(cd /tmp/box64-staging && tar cf - usr/local/bin/box64) | zstd -19 > \
    installer/app/src/main/assets/box64.tar.zst
```

### 3. Cross-Compile flash-host

The flash-host binary is x86_64 Linux, because it uses `dlopen` to load
`libpepflashplayer.so` (an x86_64 shared library) directly.  On ARM64 Android
devices, Box64 translates the entire x86_64 process to ARM64 at runtime.

The binary runs inside the PRoot chroot which has a full x86_64 glibc, so it
can be linked dynamically against glibc or statically against musl.

**Option A: Native build (on x86_64 host)**

```bash
cargo build --release -p player-android
```

**Option B: Static linking with musl**

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p player-android
```

Rename and place in jniLibs (Android extracts native libs automatically):
```bash
# For native build:
cp target/release/flash-host \
   installer/app/src/main/jniLibs/arm64-v8a/libflash-host.so

# For musl build:
cp target/x86_64-unknown-linux-musl/release/flash-host \
   installer/app/src/main/jniLibs/arm64-v8a/libflash-host.so
```

Note: The binary is named `libflash-host.so` so Android's package manager
extracts it from the APK alongside the real `.so` libraries.  The app's
`ContainerManager` copies it to `rootfs/opt/flash/flash-host`.

### 4. Obtain libpepflashplayer.so

The Adobe Flash PPAPI plugin (`libpepflashplayer.so`, x86_64) is in this folder (build-rootfs).

Place it so the user can supply it, or bundle it:
```bash
# If bundling in the rootfs archive:
cp libpepflashplayer.so build-rootfs/output/  # Before building rootfs
# Or install at runtime via ContainerManager.installFileToChroot()
```

### 5. Build the Android App

```bash
cd installer/
./gradlew assembleRelease
```

The APK will be at `installer/app/build/outputs/apk/release/`.

## Runtime Architecture

```
Android App (Kotlin)
  │
  ├── ContainerManager.initialize()
  │     ├── Extract rootfs.tar.zst → filesDir/rootfs/
  │     ├── Extract box64-v*.tar.zst → rootfs/usr/local/bin/box64
  │     └── Copy libflash-host.so → rootfs/opt/flash/flash-host
  │
  ├── IpcServer.start()  (AF_UNIX socket)
  │
  └── ContainerManager.launchHost()
        │
        └── libproot.so --rootfs=... --bind=/dev,/proc,/sys
              └── /usr/bin/env BOX64_DYNAREC=1 ... box64 /opt/flash/flash-host
                    │                         (Box64 translates x86_64→ARM64)
                    ├── Connects to IPC socket
                    ├── Creates FlashPlayer
                    ├── dlopen(libpepflashplayer.so)  ← x86_64, native to Box64
                    └── Main loop: poll → render → audio → input
```

## Directory Layout (on device)

```
/data/data/org.cleanflash.android/files/
├── rootfs/                     # Ubuntu chroot
│   ├── lib/x86_64-linux-gnu/  # x86_64 glibc for plugin
│   ├── lib/aarch64-linux-gnu/ # aarch64 glibc for Box64
│   ├── usr/local/bin/box64    # Box64 translator
│   ├── opt/flash/
│   │   ├── flash-host          # Our x86_64 host binary (runs under Box64)
│   │   └── libpepflashplayer.so # Adobe plugin (x86_64)
│   ├── home/flash/.flash/      # User data
│   └── tmp/flash/              # Session temp files
├── sockets/                    # IPC Unix sockets
└── tmp/                        # PRoot temp directory
```

## Environment Variables

### Guest (inside chroot, set via /usr/bin/env)

| Variable | Value | Purpose |
|----------|-------|---------|
| FLASH_IPC_SOCKET | /path/to/control.sock | IPC socket path |
| FLASH_SWF_URL | (URL or file://) | SWF to load |
| FLASH_SWF_WIDTH | 800 | Display width |
| FLASH_SWF_HEIGHT | 600 | Display height |
| BOX64_DYNAREC | 1 | Enable dynamic recompilation |
| BOX64_AVX | 1 | AVX instruction support |
| BOX64_LD_LIBRARY_PATH | /lib/x86_64-linux-gnu | Where Box64 finds x86_64 libs |

### Host (outside chroot, for PRoot)

| Variable | Value | Purpose |
|----------|-------|---------|
| PROOT_TMP_DIR | /path/to/tmp | PRoot temp directory |
| PROOT_LOADER | /path/to/libproot-loader.so | PRoot ELF loader |
