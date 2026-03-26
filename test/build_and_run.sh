#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "=== Flash Player Test Suite ==="
echo ""

# 1. Compile the SWFs
echo "[1/3] Compiling SWFs ..."

MXMLC_COMMON="-target-player=32.0 -swf-version=44 -static-link-runtime-shared-libraries=true"

echo "  LoadableChild.as → LoadableChild.swf"
mxmlc LoadableChild.as \
    -output LoadableChild.swf \
    -default-size 200 100 \
    $MXMLC_COMMON \
    2>&1

if [ ! -f LoadableChild.swf ]; then
    echo "ERROR: LoadableChild.swf compilation failed."
    exit 1
fi
echo "   ✓ LoadableChild.swf $(du -h LoadableChild.swf | cut -f1)"

echo "  URLLoaderTests.as → URLLoaderTests.swf"
mxmlc URLLoaderTests.as \
    -output URLLoaderTests.swf \
    -default-size 1000 700 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f URLLoaderTests.swf ]; then
    echo "ERROR: URLLoaderTests.swf compilation failed."
    exit 1
fi
echo "   ✓ URLLoaderTests.swf $(du -h URLLoaderTests.swf | cut -f1)"

echo "  FileChooserTests.as → FileChooserTests.swf"
mxmlc FileChooserTests.as \
    -output FileChooserTests.swf \
    -default-size 800 600 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f FileChooserTests.swf ]; then
    echo "ERROR: FileChooserTests.swf compilation failed."
    exit 1
fi
echo "   ✓ FileChooserTests.swf $(du -h FileChooserTests.swf | cut -f1)"

echo "  CursorLockTests.as → CursorLockTests.swf"
mxmlc CursorLockTests.as \
    -output CursorLockTests.swf \
    -default-size 800 600 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f CursorLockTests.swf ]; then
    echo "ERROR: CursorLockTests.swf compilation failed."
    exit 1
fi
echo "   ✓ CursorLockTests.swf $(du -h CursorLockTests.swf | cut -f1)"

echo "  FullscreenTests.as → FullscreenTests.swf"
mxmlc FullscreenTests.as \
    -output FullscreenTests.swf \
    -default-size 800 600 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f FullscreenTests.swf ]; then
    echo "ERROR: FullscreenTests.swf compilation failed."
    exit 1
fi
echo "   ✓ FullscreenTests.swf $(du -h FullscreenTests.swf | cut -f1)"

echo "  Stage3DTests.as → Stage3DTests.swf"
mxmlc Stage3DTests.as \
    -output Stage3DTests.swf \
    -default-size 800 600 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f Stage3DTests.swf ]; then
    echo "ERROR: Stage3DTests.swf compilation failed."
    exit 1
fi
echo "   ✓ Stage3DTests.swf $(du -h Stage3DTests.swf | cut -f1)"

echo "  URLRewriteTests.as → URLRewriteTests.swf"
mxmlc URLRewriteTests.as \
    -output URLRewriteTests.swf \
    -default-size 800 600 \
    -default-background-color=0x1e1e2e \
    $MXMLC_COMMON \
    2>&1

if [ ! -f URLRewriteTests.swf ]; then
    echo "ERROR: URLRewriteTests.swf compilation failed."
    exit 1
fi
echo "   ✓ URLRewriteTests.swf $(du -h URLRewriteTests.swf | cut -f1)"

# 2. Kill any pre-existing servers on our ports
echo ""
echo "[2/3] Starting test servers..."
for port in 3000 3001 3002; do
    pid=$(lsof -ti :$port 2>/dev/null || true)
    if [ -n "$pid" ]; then
        echo "   Killing existing process on port $port (PID $pid)"
        kill $pid 2>/dev/null || true
        sleep 0.3
    fi
done

# Start servers in background
python3 servers.py &
SERVER_PID=$!
sleep 1

# Verify they're running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "ERROR: Server failed to start."
    exit 1
fi
echo "   ✓ Servers running (PID $SERVER_PID)"

# 3. Open browser
echo ""
echo "[3/3] Ready!"
echo ""
echo "   Open: http://localhost:3000/"
echo "   Tests:"
echo "     http://localhost:3000/                 URLLoader tests"
echo "     http://localhost:3000/filechooser      FileChooser tests"
echo "     http://localhost:3000/cursorlock        Cursor Lock tests"
echo "     http://localhost:3000/fullscreen        Fullscreen tests"
echo "     http://localhost:3000/stage3d            Stage3D tests"
echo "     http://localhost:3000/urlrewrite          URL Rewrite tests"
echo ""
echo "   Press Ctrl+C to stop servers."

# Wait for Ctrl+C
trap "echo ''; echo 'Stopping servers...'; kill $SERVER_PID 2>/dev/null; exit 0" INT TERM
wait $SERVER_PID
