# Dockerfile for building a minimal x86_64 rootfs for flash-host + Box64
#
# This produces a tar archive containing only the libraries needed to run
# libpepflashplayer.so under Box64 on ARM64 Android.
#
# Build:
#   docker build -t flash-rootfs-builder .
#   docker run --rm flash-rootfs-builder > rootfs.tar
#
# Compress for APK assets:
#   zstd -19 rootfs.tar -o rootfs.tar.zst

FROM ubuntu:20.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive

# Install debootstrap and tools
RUN apt-get update && apt-get install -y \
    debootstrap \
    zstd \
    && rm -rf /var/lib/apt/lists/*

# Create a minimal x86_64 rootfs using debootstrap
RUN debootstrap \
    --arch=amd64 \
    --variant=minbase \
    --include=libc6,libstdc++6,libgcc-s1 \
    focal /rootfs-full http://archive.ubuntu.com/ubuntu

# Now strip down to only what we need
RUN mkdir -p /rootfs

# ---- x86_64 libraries (for libpepflashplayer.so via Box64) ----
RUN mkdir -p /rootfs/lib/x86_64-linux-gnu && \
    for lib in \
        libc.so.6 \
        libdl.so.2 \
        librt.so.1 \
        libpthread.so.0 \
        libm.so.6 \
        libc-2.31.so \
        libdl-2.31.so \
        librt-2.31.so \
        libpthread-2.31.so \
        libm-2.31.so \
        libgcc_s.so.1 \
        libstdc++.so.6 \
        libstdc++.so.6.0.28 \
    ; do \
        [ -e "/rootfs-full/lib/x86_64-linux-gnu/$lib" ] && \
            cp -a "/rootfs-full/lib/x86_64-linux-gnu/$lib" /rootfs/lib/x86_64-linux-gnu/ ; \
        [ -e "/rootfs-full/usr/lib/x86_64-linux-gnu/$lib" ] && \
            cp -a "/rootfs-full/usr/lib/x86_64-linux-gnu/$lib" /rootfs/lib/x86_64-linux-gnu/ ; \
    done

# x86_64 dynamic linker
RUN mkdir -p /rootfs/lib64 && \
    cp -a /rootfs-full/lib/x86_64-linux-gnu/ld-2.31.so /rootfs/lib/x86_64-linux-gnu/ && \
    cp -a /rootfs-full/lib64/ld-linux-x86-64.so.2 /rootfs/lib64/

# ---- Essential directories ----
RUN mkdir -p \
    /rootfs/bin \
    /rootfs/etc \
    /rootfs/tmp/flash \
    /rootfs/home/flash/.flash/storage \
    /rootfs/opt/flash \
    /rootfs/usr/bin \
    /rootfs/usr/lib \
    /rootfs/dev \
    /rootfs/proc \
    /rootfs/sys

# /usr/bin/env is needed by PRoot command line
RUN cp /rootfs-full/usr/bin/env /rootfs/usr/bin/env && \
    chmod 755 /rootfs/usr/bin/env

# Minimal /etc files
RUN echo "root:x:0:0:root:/root:/bin/sh" > /rootfs/etc/passwd && \
    echo "root:x:0:" > /rootfs/etc/group && \
    echo "flash:x:1000:1000:Flash Player:/home/flash:/bin/sh" >> /rootfs/etc/passwd && \
    echo "flash:x:1000:" >> /rootfs/etc/group && \
    echo "nameserver 8.8.8.8" > /rootfs/etc/resolv.conf && \
    echo "127.0.0.1 localhost" > /rootfs/etc/hosts

# ---- NSS libraries (needed for getaddrinfo/DNS resolution) ----
RUN for lib in \
        libnss_dns.so.2 libnss_dns-2.31.so \
        libnss_files.so.2 libnss_files-2.31.so \
        libresolv.so.2 libresolv-2.31.so \
    ; do \
        [ -e "/rootfs-full/lib/x86_64-linux-gnu/$lib" ] && \
            cp -a "/rootfs-full/lib/x86_64-linux-gnu/$lib" /rootfs/lib/x86_64-linux-gnu/ ; \
    done

# nsswitch.conf for name resolution
RUN echo "passwd: files" > /rootfs/etc/nsswitch.conf && \
    echo "group: files" >> /rootfs/etc/nsswitch.conf && \
    echo "hosts: files dns" >> /rootfs/etc/nsswitch.conf

# ---- ld.so configuration ----
RUN mkdir -p /rootfs/etc/ld.so.conf.d && \
    echo "/lib/x86_64-linux-gnu" > /rootfs/etc/ld.so.conf && \
    echo "/usr/lib/x86_64-linux-gnu" >> /rootfs/etc/ld.so.conf

# ---- aarch64 libraries (for Box64 itself, which is an ARM64 binary) ----
# Box64 is aarch64 and needs aarch64 glibc to run.  Inside the PRoot
# chroot these live alongside the x86_64 libs.
# Box64 from Winlator requires GLIBC_2.39, so we use Ubuntu Noble (24.04).
# Note: glibc 2.34+ merged libdl/libpthread/librt into libc.so.6.
RUN debootstrap \
    --arch=arm64 \
    --variant=minbase \
    --include=libc6,libgcc-s1 \
    --foreign \
    noble /rootfs-arm64 http://ports.ubuntu.com/ubuntu-ports
RUN mkdir -p /rootfs/lib/aarch64-linux-gnu && \
    for lib in \
        libc.so.6 \
        libm.so.6 \
        libresolv.so.2 \
        libgcc_s.so.1 \
        ld-linux-aarch64.so.1 \
        libdl.so.2 \
        libpthread.so.0 \
        librt.so.1 \
    ; do \
        [ -e "/rootfs-arm64/lib/aarch64-linux-gnu/$lib" ] && \
            cp -a "/rootfs-arm64/lib/aarch64-linux-gnu/$lib" /rootfs/lib/aarch64-linux-gnu/ ; \
        [ -e "/rootfs-arm64/usr/lib/aarch64-linux-gnu/$lib" ] && \
            cp -a "/rootfs-arm64/usr/lib/aarch64-linux-gnu/$lib" /rootfs/lib/aarch64-linux-gnu/ ; \
    done

# ---- Placeholder directories for Box64 and flash-host ----
# Box64 binary will be extracted separately (versioned archive)
# flash-host binary will be copied from the Android app's native libs

# ---- Bundle libpepflashplayer.so (must be in build context) ----
COPY libpepflashplayer.so /rootfs/opt/flash/libpepflashplayer.so

# Create the output tar inside the builder stage
RUN cd /rootfs && tar cf /rootfs.tar .
