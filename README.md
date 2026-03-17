# Clean Flash Player

Clean Flash Player is a browser extension that brings Adobe Flash Player back to Google Chrome and Mozilla Firefox using a sandboxed, out-of-process architecture built entirely in Rust.

It also provides a standalone PPAPI host library and desktop players, enabling Flash content (or any PPAPI plugin) to run outside the browser.

## Browser Extension

The core of the project is a Manifest V3 browser extension that transparently detects Flash content on web pages and replaces each instance with a live canvas driven by a native PPAPI host process.

### How It Works

1. **Page script** spoofs the web browser plugin list so that existing feature-detection code (including swfobject) continues to find Flash.
2. **Content script** scans the DOM for Flash embeds and replaces them with a rendering canvas.
3. **Background service worker** spawns a **Native Messaging** host process (`flash-player-host`) for each Flash instance.
4. **Native host** (`player-web` crate) loads the PPAPI Flash plugin and streams messages back to the extension over Native Messaging.

The extension also implements a full **ExternalInterface** bridge so that page JavaScript can call into Flash and vice-versa, matching the historical `<object>`/`<embed>` scripting contract.

### Supported Browsers

| Browser | Supported |
|---------|-----------|
| Google Chrome | ✅ |
| Chromium | ✅ |
| Brave | ✅ |
| Mozilla Firefox (v111+) | ❌ |

### Installation

1. Build the native messaging host:
   ```bash
   cargo build --release -p player-web
   ```
2. Install the native messaging host manifest for your browser(s):
   ```bash
   # Linux
   cd web-extension && bash install-host.sh

   # Linux & Windows (Python)
   cd web-extension && python install-host.py
   ```
3. Load the extension:
   - **Chrome/Chromium/Brave:** go to `chrome://extensions`, enable Developer Mode, click "Load unpacked", and select the `web-extension/` directory.
   - **Firefox:** go to `about:debugging#/runtime/this-firefox`, click "Load Temporary Add-on", and select `web-extension/manifest.json`.

## Sandboxing

The native host process is sandboxed **after** the plugin (and all required shared libraries) have been loaded. This ensures that `dlopen`, GPU driver initialization, and other setup calls succeed before the syscall surface is restricted.

### Linux - seccomp-BPF

On x86_64 Linux the host installs a seccomp-BPF filter that:

- **Blocks `execve` / `execveat`** - the plugin cannot spawn child processes.
- **Blocks `mmap` with `PROT_EXEC`** - prevents loading new executable code from disk.
- **Allows `mprotect` with `PROT_EXEC`** - required for Flash Player's JIT compiler.
- **Blocks `memfd_create`** - prevents creation of anonymous executable memory-backed file descriptors.

The filter is applied per-thread (no `TSYNC`), which allows dedicated worker threads (e.g. the file-chooser dialog thread) to be spawned **before** sandbox activation and remain unsandboxed for operations that require blocked syscalls.

`PR_SET_NO_NEW_PRIVS` is set before the seccomp filter is installed to prevent privilege escalation.

### Windows - Job Objects & Privilege De-escalation

On Windows the host de-escalates the process by creating a Job Object and lowering process privileges.

## PPAPI Host Library

The `ppapi-host` crate is a general-purpose, embeddable PPAPI host implementation written in Rust. It can be used independently of the browser extension.

### Use Cases

| Use case | How |
|----------|-----|
| **Embed any PPAPI plugin anywhere** | Link `ppapi-host`, implement the provider traits from `player-ui-traits`, and call `load_plugin` / `create_instance`. |
| **Embed Flash Player anywhere** | Use the `player-core` crate which wraps `ppapi-host` with Flash-specific lifecycle management (SWF loading, `HandleDocumentLoad`, input dispatch, frame streaming). |
| **Embed Flash Player in the browser** | Use the browser extension + `player-web` native host described above. |

### Feature Flags (`ppapi-host`)

| Feature | Description |
|---------|-------------|
| `fs-os` | Native filesystem access |
| `fs-memory` | In-memory filesystem |
| `fs-stub` | Stub filesystem (no-op) |
| `audio-cpal` | Audio output via cpal |
| `clipboard-arboard` | Clipboard via arboard |
| `url-reqwest` | URL loading via reqwest |
| `url-stub` | Stub URL loader (no-op) |

### Desktop Players

- **player-egui** - Cross-platform desktop player using egui/eframe. Supports file dialogs, context menus, fullscreen, clipboard, audio, and optional webcam capture.
- **player-win32** - Native Win32 desktop player using GDI rendering and the Windows API directly.

Build a desktop player:

```bash
# egui (cross-platform)
cargo build --release -p player-egui

# Win32 (Windows only)
cargo build --release -p player-win32
```

## Testing

The `test/` directory contains ActionScript test cases and an HTTP test harness:

```bash
cd test && bash build_and_run.sh
```
