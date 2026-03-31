#!/usr/bin/env bash
#
# Build a minimal x86_64 rootfs for the Flash Player Android app.
#
# This script builds the rootfs using Docker for reproducibility,
# then compresses it with zstd for inclusion in the APK assets.
#
# Usage:
#   ./build.sh                    # Build rootfs.tar.zst
#   ./build.sh --with-box64       # Also download and bundle Box64
#   ./build.sh --output DIR       # Output to a specific directory
#
# Requirements:
#   - Docker
#   - zstd (for compression)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Defaults
OUTPUT_DIR="$SCRIPT_DIR/output"
WITH_BOX64=false
BOX64_VERSION="0.2.8"
BOX64_ARCH="aarch64"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --with-box64)
            WITH_BOX64=true
            shift
            ;;
        --box64-version)
            BOX64_VERSION="$2"
            shift 2
            ;;
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--with-box64] [--box64-version VER] [--output DIR]"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

mkdir -p "$OUTPUT_DIR"

echo "=== Building minimal x86_64 rootfs ==="

# Build the Docker image
docker build -t flash-rootfs-builder "$SCRIPT_DIR"

# Export the rootfs tar that was built inside the container.
# (We copy the pre-built /rootfs.tar rather than using `docker export`,
# which would dump the entire builder filesystem including the full
# debootstrap sysroot.)
echo "=== Exporting rootfs ==="
CONTAINER_ID=$(docker create flash-rootfs-builder)
docker cp "$CONTAINER_ID:/rootfs.tar" "$OUTPUT_DIR/rootfs.tar"
docker rm "$CONTAINER_ID" > /dev/null

echo "Rootfs tar: $(du -sh "$OUTPUT_DIR/rootfs.tar" | cut -f1)"

# Compress with zstd (level 19 for best compression)
echo "=== Compressing with zstd ==="
zstd -19 --rm -f "$OUTPUT_DIR/rootfs.tar" -o "$OUTPUT_DIR/rootfs.tar.zst"
echo "Compressed: $(du -sh "$OUTPUT_DIR/rootfs.tar.zst" | cut -f1)"

# Optionally download and package Box64
if $WITH_BOX64; then
    echo "=== Downloading Box64 v${BOX64_VERSION} ==="

    BOX64_DIR="$OUTPUT_DIR/box64-staging"
    mkdir -p "$BOX64_DIR/usr/bin"

    # Download pre-built Box64 for aarch64
    BOX64_URL="https://github.com/ptitSeb/box64/releases/download/v${BOX64_VERSION}/box64-${BOX64_ARCH}"

    if command -v curl &>/dev/null; then
        curl -L -o "$BOX64_DIR/usr/bin/box64" "$BOX64_URL"
    elif command -v wget &>/dev/null; then
        wget -O "$BOX64_DIR/usr/bin/box64" "$BOX64_URL"
    else
        echo "ERROR: Need curl or wget to download Box64" >&2
        exit 1
    fi

    chmod 755 "$BOX64_DIR/usr/bin/box64"

    # Create Box64 archive
    echo "=== Packaging Box64 ==="
    (cd "$BOX64_DIR" && tar cf - usr/bin/box64) | zstd -19 > "$OUTPUT_DIR/box64-v${BOX64_VERSION}.tar.zst"
    echo "Box64 archive: $(du -sh "$OUTPUT_DIR/box64-v${BOX64_VERSION}.tar.zst" | cut -f1)"

    rm -rf "$BOX64_DIR"
fi

echo ""
echo "=== Build complete ==="
echo "Output files in $OUTPUT_DIR:"
ls -lh "$OUTPUT_DIR"/*.tar.zst 2>/dev/null || true

echo ""
echo "To use in the Android app:"
echo "  1. Copy rootfs.tar.zst to installer/app/src/main/assets/"
if $WITH_BOX64; then
    echo "  2. Copy box64-v${BOX64_VERSION}.tar.zst to installer/app/src/main/assets/"
fi
echo ""
echo "The rootfs contains:"
echo "  - x86_64 glibc + libstdc++ + libgcc (for libpepflashplayer.so)"
echo "  - /usr/bin/env (needed by PRoot command line)"
echo "  - Minimal /etc (passwd, group, resolv.conf, hosts, nsswitch.conf)"
echo "  - NSS libraries (DNS resolution support)"
echo "  - Directory structure: /opt/flash/, /home/flash/, /tmp/flash/"
