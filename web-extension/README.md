# Flash Player Web Extension

A Chrome/Firefox extension that detects Flash content (`<object>` and `<embed>` elements) on web pages and replaces them with a native `<canvas>` powered by the Flash Player Native Messaging host.

## Architecture

```
┌──────────────┐      ┌──────────────────┐      ┌─────────────────────┐
│  Web Page    │      │  Background      │      │  flash-player-host  │
│  content.js  │◄────►│  background.js   │◄────►│  (Rust binary)      │
│  <canvas>    │ port │  Service Worker   │stdio │  player-core        │
└──────────────┘      └──────────────────┘      └─────────────────────┘
```

- **content.js** - Injected into every page. Uses a `MutationObserver` to find Flash `<object>`/`<embed>` elements and replaces them with a `<canvas>`. Input events on the canvas are forwarded through the background worker to the native host. Frame updates (dirty subregions only) are drawn onto the canvas.

- **background.js** - Service worker that manages the Native Messaging connection to the `flash-player-host` binary. Bridges messages between content scripts and the native host.

- **flash-player-host** - The Rust binary (`player-web` crate) that hosts the PepperFlash PPAPI plugin. Communicates via the Native Messaging protocol (length-prefixed JSON on stdin/stdout).

## Setup

### 1. Build the native host

```bash
cargo build --release -p player-web
```

The binary will be at `target/release/flash-player-host` on Linux and `target/release/flash-player-host.exe` on Windows.

### 2. Install the native messaging manifest

```bash
cd web-extension
python install-host.py ../target/release/flash-player-host
```

On Windows, run `py -3 install-host.py ..\target\release\flash-player-host.exe` instead.

This installs the JSON manifest to the standard locations for Firefox, Chrome, Chromium, and Brave on Linux, and writes the corresponding per-user registry entries on Windows.

### 3. Set the Flash plugin path

```bash
export FLASH_PLUGIN_PATH=/path/to/libpepflashplayer.so
```

On Windows, set `FLASH_PLUGIN_PATH` to the PepperFlash `.dll` path before launching the browser.

### 4. Load the extension

**Chrome/Chromium:**
1. Go to `chrome://extensions`
2. Enable "Developer mode"
3. Click "Load unpacked" and select the `web-extension/` directory

**Firefox:**
1. Go to `about:debugging#/runtime/this-firefox`
2. Click "Load Temporary Add-on"
3. Select `web-extension/manifest.json`

## Protocol

### Extension → Host (JSON over stdin)

| Type        | Fields                                                    |
|-------------|-----------------------------------------------------------|
| `open`      | `url`                                                     |
| `resize`    | `width`, `height`                                         |
| `mousedown` | `x`, `y`, `button`, `modifiers`                           |
| `mouseup`   | `x`, `y`, `button`, `modifiers`                           |
| `mousemove` | `x`, `y`, `modifiers`                                     |
| `keydown`   | `keyCode`, `code`, `modifiers`                            |
| `keyup`     | `keyCode`, `code`, `modifiers`                            |
| `char`      | `keyCode`, `text`, `code`, `modifiers`                    |
| `wheel`     | `deltaX`, `deltaY`, `modifiers`                           |
| `focus`     | `hasFocus`                                                |
| `close`     | _(none)_                                                  |

### Host → Extension (JSON over stdout)

| Type     | Fields                                                                |
|----------|-----------------------------------------------------------------------|
| `frame`  | `x`, `y`, `width`, `height`, `frameWidth`, `frameHeight`, `data` (base64 BGRA) |
| `state`  | `state` (`"idle"`, `"running"`), optional `width`, `height`           |
| `cursor` | `cursor` (PP_CursorType_Dev integer)                                  |
| `error`  | `message`                                                             |
