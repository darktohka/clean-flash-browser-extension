# URLLoader & URLLoaderTrusted Implementation Plan

## 1. How freshplayerplugin's URLLoader Works

### Architecture
freshplayerplugin routes all URL loading through the browser's **NPAPI** layer. The URLLoader state is a C struct (`pp_url_loader_s`) containing:
- Response metadata: `status_line`, `headers`, `http_code`, `redirect_url`, `response_size`
- Streaming state: a temp file (`fd`), a `read_pos` cursor, and a `finished_loading` flag
- An `NPStream*` binding to the browser's active stream
- A `GList *read_tasks` queue of pending `ReadResponseBody` operations
- Request config copied from `URLRequestInfo` at open time

### Request Flow
1. **Open**: Plugin calls `ppb_url_loader_open()`. Config is copied from URLRequestInfo. A temp file is created (`/tmp/FreshStreamXXXXXX`). An NPAPI `geturlnotify`/`posturlnotify` call is dispatched to the browser thread.
2. **Headers arrive**: Browser calls `NPP_NewStream()`. Headers are parsed (`hp_parse_headers`). If it's a redirect (3xx) and `follow_redirects` is true, `ppb_url_loader_follow_redirect` is called automatically. Otherwise, the open callback fires with PP_OK.
3. **Data arrives**: Browser calls `NPP_Write()`. Data is written to the temp file. If there's a pending ReadResponseBody task in the queue, it's dequeued and its buffer is filled from the file, and its callback fires.
4. **Stream ends**: Browser calls `NPP_DestroyStream()`. `finished_loading` is set to 1. All remaining queued read tasks are flushed (returning 0 bytes for EOF).
5. **ReadResponseBody**: If data is already in the temp file past `read_pos`, reads synchronously and returns byte count. Otherwise, queues a read task and returns `PP_OK_COMPLETIONPENDING`.

### Redirect Handling
- `FollowRedirect`: Clears old response state, extracts `redirect_url`, forces method to GET, clears POST body, opens a fresh temp file, re-issues the NPAPI request.
- When `follow_redirects=true`, redirects are followed automatically inside `NPP_NewStream`.

### Progress Tracking
- **Download**: `fstat(fd).st_size` for bytes received, `response_size` from Content-Length for total.
- **Upload**: Always returns `PP_FALSE` (NPAPI provides no upload progress mechanism).

### Limitations
- Tied to NPAPI browser callbacks (no standalone HTTP client)
- No upload progress tracking
- No redirect loop detection
- No concurrency limiting

---

## 2. How Chrome's URLLoader Works

### Architecture (Two-Process Model)
Chrome splits the implementation across two processes via IPC:

**Plugin-side** (`URLLoaderResource` in `ppapi/proxy/url_loader_resource.cc`):
- State machine: `MODE_WAITING_TO_OPEN → MODE_OPENING → MODE_STREAMING_DATA → MODE_LOAD_COMPLETE`
- Internal buffer: `circular_deque<char>` for received data
- Backpressure: Suspends Blink's loader when buffer exceeds upper threshold (512KB), resumes at lower threshold (128KB)
- Single pending callback model: `pending_callback_` — only one async operation at a time

**Browser-side** (`PepperURLLoaderHost` in `content/renderer/pepper/pepper_url_loader_host.cc`):
- Wraps `blink::WebAssociatedURLLoader` for actual network I/O
- Implements `WebAssociatedURLLoaderClient` callbacks: `WillFollowRedirect`, `DidSendData`, `DidReceiveResponse`, `DidDownloadData`, `DidReceiveData`, `DidFinishLoading`, `DidFail`
- Message ordering: `ReceivedResponse → SendData* → FinishedLoading`

### Request Flow
1. **Create**: Plugin creates `URLLoaderResource` in `MODE_WAITING_TO_OPEN`.
2. **Open**: Plugin sends `PpapiHostMsg_URLLoader_Open`. Browser validates access, creates `WebURLRequest`, calls `LoadAsynchronously()`. Mode → `MODE_OPENING`.
3. **Response**: Browser's `DidReceiveResponse` → `SaveResponse()` → sends `ReceivedResponse` message → plugin fires open callback.
4. **Data**: Browser's `DidReceiveData` → sends `SendData` messages → plugin appends to `circular_deque`. If `user_buffer_` set, fills and fires callback.
5. **Completion**: Browser's `DidFinishLoading` → sends `FinishedLoading(PP_OK)` → plugin sets `MODE_LOAD_COMPLETE`.
6. **ReadResponseBody**: If buffer has data, copies to user buffer and returns byte count. If `done_status_` set, returns it (0 or error). Otherwise, registers callback and returns `PP_OK_COMPLETIONPENDING`.

### Redirect Handling
- `WillFollowRedirect`: If `follow_redirects=false`, saves response info and calls `SetDefersLoading(true)` to pause the load. Plugin can inspect the response, then call `FollowRedirect` which calls `SetDefersLoading(false)`.
- If `follow_redirects=true`, returns `true` (allows Blink to follow automatically).
- Cross-origin 307/308 POST redirects are blocked (Firefox compatibility).

### Progress Tracking
- **Upload**: `DidSendData(bytes_sent, total)` updates counters.
- **Download**: `DidReceiveData` and `DidDownloadData` increment `bytes_received_`. `total_bytes_to_be_received_` set from `ExpectedContentLength()`.
- `UpdateProgress()` sends `PpapiPluginMsg_URLLoader_UpdateProgress` to plugin if `record_download_progress` or `record_upload_progress` is set.
- Status callback (`PP_URLLoaderTrusted_StatusCallback`) invoked on every progress update.

### Trusted Interface
- `GrantUniversalAccess`: Requires `PERMISSION_PDF` or `PERMISSION_FLASH`. Sets `has_universal_access_=true` which grants `grant_universal_access` in `WebAssociatedURLLoaderOptions`, bypassing CORS.
- `RegisterStatusCallback`: Stores a function pointer called on every progress update with `(instance, resource, bytes_sent, total_sent, bytes_received, total_recv)`.

### Security
- Same-origin by default; cross-origin requires `allow_cross_origin_requests`.
- Universal access bypasses all origin checks (Flash/PDF only).
- Service workers are skipped for plugin requests.

---

## 3. Implementation Plan

### Key Design Decisions
1. **No IPC split**: Since we host the plugin in-process, we combine the plugin-side and browser-side into a single `URLLoaderResource` struct.
2. **Tokio runtime**: Use the existing shared tokio runtime (`ppapi_host::tokio_runtime()`) for HTTP I/O.
3. **HostCallbacks::on_url_open**: The existing trait method does single-shot request/response. We'll replace this with a more capable internal HTTP client that handles redirects and progress natively.
4. **reqwest**: The workspace already depends on `reqwest` (with `blocking` + `rustls` features). We'll use its async API via tokio for streaming.
5. **Concurrency**: Global `tokio::sync::Semaphore` with 8 permits for limiting simultaneous requests.

### File Structure
All code goes in one new file: `crates/ppapi-host/src/interfaces/url_loader.rs`

### Resource Struct

```rust
pub struct URLLoaderResource {
    instance: PP_Instance,
    mode: Mode,  // WaitingToOpen, Opening, StreamingData, LoadComplete

    // Request state (copied from URLRequestInfo on Open)
    request_data: URLRequestData,

    // Response state
    response_info_id: Option<PP_Resource>,  // Created lazily
    status_code: i32,
    status_line: String,
    headers: String,
    redirect_url: String,
    response_url: String,

    // Streaming buffer (replaces Chrome's circular_deque)
    buffer: VecDeque<u8>,
    done_status: Option<i32>,  // None = still loading, Some(PP_OK) = done, Some(err) = failed

    // Progress tracking
    bytes_sent: i64,
    total_bytes_to_be_sent: i64,
    bytes_received: i64,
    total_bytes_to_be_received: i64,

    // Trusted interface state
    has_universal_access: bool,
    status_callback: PP_URLLoaderTrusted_StatusCallback,

    // Pending callback for Open/FollowRedirect/ReadResponseBody
    // (stored as raw PP_CompletionCallback since we fire from background thread)
    pending_callback: Option<PP_CompletionCallback>,

    // ReadResponseBody pending state
    user_buffer: Option<(*mut u8, usize)>,  // (ptr, max_bytes)

    // Abort handle to cancel in-flight request
    abort_handle: Option<tokio::sync::oneshot::Sender<()>>,

    // Redirect tracking for loop detection
    redirect_chain: Vec<String>,
}
```

### Implementation Steps

#### Step 1: Create URLLoaderResource + basic vtable
- Define `URLLoaderResource` struct implementing `Resource` trait
- Define `PPB_URLLoader_1_0` static vtable with all function pointers
- Register with `PPB_URLLOADER_INTERFACE_1_0` string
- Implement `Create`, `IsURLLoader`, `Close`

#### Step 2: Implement `Open`
- Extract fields from `URLRequestInfoResource` via `with_downcast`
- Validate state (must be `MODE_WAITING_TO_OPEN`)
- Copy request config, save callback
- Spawn tokio task:
  1. Acquire semaphore permit (blocks if 8 concurrent requests active)
  2. Build reqwest Request (method, URL, headers, body)
  3. If `follow_redirects=true`: Follow redirects internally (with loop detection: track visited URLs, limit to 20 hops)
  4. If `follow_redirects=false`: Use `reqwest::redirect::Policy::none()` to capture first redirect
  5. On response: Extract headers, status, content-length → update resource
  6. Create `URLResponseInfoResource` and store its ID
  7. Begin streaming body chunks into VecDeque buffer
  8. Fire open callback with PP_OK on main thread via MessageLoopPoster
  9. Continue appending chunks until done, then set `done_status`

#### Step 3: Implement `ReadResponseBody`
- Validate: response must exist, no pending callback, bytes_to_read > 0
- If buffer has data: copy to user buffer, return byte count
- If `done_status` is set and buffer empty: return done_status (0 for clean EOF → PP_OK mapped to 0)
- Otherwise: save user_buffer pointer + pending_callback, return `PP_OK_COMPLETIONPENDING`
- Background streaming task checks after each chunk: if user_buffer is set, fill it and fire callback

#### Step 4: Implement `FollowRedirect`
- Validate: must be in MODE_OPENING with a redirect response
- Clear old response, save callback
- Re-open the redirect_url (reuse Open logic, but with GET method, no body)
- Add redirect_url to redirect_chain for loop detection

#### Step 5: Implement Progress
- `GetUploadProgress`: Return bytes_sent / total_bytes_to_be_sent if `record_upload_progress`
- `GetDownloadProgress`: Return bytes_received / total_bytes_to_be_received if `record_download_progress`
- Background task updates counters as data streams
- If `status_callback` is set, invoke it on progress updates

#### Step 6: Implement `GetResponseInfo`
- Create `URLResponseInfoResource` if not already created
- AddRef and return its PP_Resource handle

#### Step 7: Implement URLLoaderTrusted (PPB_URLLoaderTrusted;0.3)
- `GrantUniversalAccess`: Set `has_universal_access = true` on the URLLoaderResource
- `RegisterStatusCallback`: Store the callback function pointer

#### Step 8: Concurrency Management
- Global `static SEMAPHORE: OnceLock<tokio::sync::Semaphore>` with 8 permits
- Each Open() acquires a permit before making the HTTP request
- Permit is held until the response body is fully consumed or the loader is closed
- This naturally blocks excess requests without busy-waiting

#### Step 9: Redirect Loop Detection
- Maintain `redirect_chain: Vec<String>` on each URLLoaderResource
- Before following a redirect, check if URL already in chain → `PP_ERROR_FAILED`
- Also limit total redirects to 20 (matching browser behavior)

### Thread Safety
- `URLLoaderResource` fields modified from background tokio tasks use interior mutability via the existing `ResourceManager` Mutex-based access pattern
- Callbacks are posted to the main message loop via `MessageLoopPoster`
- The `user_buffer` pointer is only written by the plugin thread and read by the background thread after the plugin is blocked waiting for a callback — no concurrent access

### Dependencies Required
- `tokio` (already in Cargo.toml with `rt-multi-thread` + `sync`)
- `reqwest` — needs to be added to ppapi-host's Cargo.toml (async API, not blocking)
- No new external crates needed

### Modifications to Existing Code
1. **ppapi-host/Cargo.toml**: Add `reqwest` dependency
2. **interfaces/mod.rs**: Already has `pub mod url_loader;` — just need to create the file
3. **interfaces/url_loader.rs**: New file — full URLLoader + URLLoaderTrusted implementation
4. **HostCallbacks::on_url_open**: Keep existing trait for backward compat; URLLoader will use reqwest directly OR delegate through on_url_open if host_callbacks is set (to support browser-hosted players that need to route through fetch API)

---

# PPB_Flash_MessageLoop & PPB_MessageLoop Implementation Plan

## 1. Problem Analysis

### PPB_Flash_MessageLoop Freeze

The trace shows:
```
PPB_Flash_MessageLoop::Create(instance=2)
PPB_Flash_MessageLoop::Run(flash_message_loop=6893)
```
…and then the entire system freezes. Flash is attempting to show an ActionScript
error dialog (or context menu). The PPAPI pattern is:

1. Flash initiates an **async** operation (show dialog/context menu via another PPB call)
2. Flash calls `Run()` to block until that async operation completes
3. In the async operation's completion callback, Flash calls `Quit()`
4. `Run()` returns, Flash continues

The freeze occurs because **`Quit()` is never called** — the underlying async
operation (dialog, context menu, etc.) either isn't implemented, or its
completion callback never fires. `Run()` blocks forever.

### Root Causes (Flash MessageLoop)

1. **Missing Quit trigger**: The PPAPI operation that would complete and call
   `Quit()` (likely `PPB_Flash_Menu::Show` or a dialog API) is stubbed or
   absent, so no completion callback ever fires.

2. **Wrong-thread callback execution**: The current implementation drains the
   main-thread message loop (`drain_ready()`) and executes those callbacks
   **on the plugin thread**. Callbacks posted via `CallOnMainThread` expect to
   run on the main thread; running them on the plugin thread violates
   thread-safety assumptions.

3. **Mutex contention risk**: Both the plugin thread (inside `run()`) and the
   main/UI thread (inside `poll_main_loop()`) race to drain the same main-loop
   channel. While not strictly deadlocking (the parking_lot Mutex is released
   before callbacks execute), executing callbacks on both threads creates data
   races.

4. **No destruction guard**: Chrome returns `PP_ERROR_ABORTED` from `Run()` when
   the resource is destroyed before `Quit()` is called. The current
   implementation has no such mechanism.

---

## 2. Chrome's Architecture (for reference)

### PPB_Flash_MessageLoop in Chrome

**File**: `content/renderer/pepper/ppb_flash_message_loop_impl.cc`

- **`Create(instance)`**: Allocates resource with internal `State` (ref-counted):
  - `run_called_` flag (only first `Run()` proceeds; subsequent return `PP_ERROR_FAILED`)
  - `quit_closure_` (a `base::OnceClosure` to break the RunLoop)
  - `result_` (set to `PP_OK` by `Quit()` or `PP_ERROR_ABORTED` by destructor)

- **`Run()`** (`InternalRun`):
  1. Checks `run_called_`; rejects duplicate calls with `PP_ERROR_FAILED`
  2. Creates `base::RunLoop(kNestableTasksAllowed)` — a **nested** event loop
  3. Stores `run_loop.QuitClosure()` in state
  4. Creates `blink::WebScopedPagePauser` to pause page rendering
  5. Runs `run_loop.Run()` — **blocks here**, but the nested RunLoop continues
     processing all Chromium tasks (timers, async I/O, IPC, DOM events)
  6. Returns `state_protector->result()` after the loop exits

- **`Quit()`** (`InternalQuit`):
  1. Sets `result_` to `PP_OK`
  2. Runs `quit_closure_` → breaks the nested RunLoop
  3. Fires `RunFromHostProxyCallback` if called via IPC proxy

- **Destructor**: Calls `InternalQuit(PP_ERROR_ABORTED)` so `Run()` unblocks

**Key**: In Chrome, `Run()` executes on the **renderer main thread** (via IPC
from the plugin process). The nested RunLoop processes ALL renderer-side tasks,
including the completion callback that will call `Quit()`.

### PPB_MessageLoop in Chrome

**File**: `ppapi/proxy/ppb_message_loop_proxy.cc`

- Background thread loops: plugin creates a `MessageLoopResource`, attaches to
  thread, calls `Run()`. `Run()` creates a `base::RunLoop` and blocks processing
  tasks.
- **Nesting**: Uses `nested_invocations_` counter; supports nested `Run()` calls
  (each has its own `base::RunLoop`).
- **PostQuit**: If on current thread, calls `run_loop_->QuitWhenIdle()`.
  Otherwise, posts a closure that does the same.
- **should_destroy**: When true + outermost Run exits, the task executor is
  destroyed and further PostWork fails.
- **Main thread**: Has an implicitly-created loop; `Run()` returns
  `PP_ERROR_INPROGRESS` for main thread.

---

## 3. Current ppapi-host Implementation Analysis

### PPB_Flash_MessageLoop (`flash_message_loop.rs`)

```
FlashMessageLoopResource {
    quit_signal: Arc<(Mutex<bool>, Condvar)>
}
```

- **`run()`**: Blocks plugin thread in a `condvar.wait_timeout(5ms)` loop.
  Every 5ms, drains the main message loop via `drain_ready()` and executes
  callbacks **on the plugin thread**. Loops until `quit_signal` is set.
- **`quit()`**: Sets quit flag, notifies condvar.
- **Problems**: See §1 above.

### PPB_MessageLoop (`message_loop.rs` + `interfaces/message_loop.rs`)

- Uses `crossbeam_channel` for work queue
- `run()` blocks on `receiver.recv()`, executes callbacks inline
- `post_quit()` sends a null-callback sentinel
- Thread-local `CURRENT_LOOP` for `GetCurrent()`
- Main-thread loop: marked via `set_main_thread_loop(true)`, polled
  externally by `poll_main_loop()` in player-core

**Issues vs Chrome**:
1. **No nesting support**: Returns `PP_ERROR_INPROGRESS` for `depth > 0`.
   Chrome allows nesting (with `nested_invocations_` counter). This may not
   matter for Flash since Flash uses `PPB_Flash_MessageLoop` for nesting, not
   `PPB_MessageLoop`.
2. **Quit semantics diff**: Chrome uses `QuitWhenIdle` (runs remaining pending
   tasks before quitting). Current impl sends a sentinel that breaks immediately,
   losing any queued-but-not-yet-processed work. Should drain remaining items
   before exiting when `should_destroy = true`.
3. **Main thread rejection differs**: Chrome returns `PP_ERROR_INPROGRESS` for
   Run on main thread; current returns `PP_ERROR_INPROGRESS` as well. ✓ Matches.

### Main Thread Polling (player-core/player-web/player-egui)

- `FlashPlayer::poll_main_loop()`: drains the main MessageLoop resource via
  `drain_ready()`, then executes callbacks (mutex released first). Called:
  - **player-web**: in a `loop {}` with `thread::sleep(4ms)` between iterations
  - **player-egui**: in `App::update()`, called every egui frame (~16ms)

---

## 4. Implementation Plan

### 4A. PPB_Flash_MessageLoop Fix

#### Approach: "Cooperative Nested Loop with Graceful Fallback"

The fundamental constraint is that ppapi-host does not have a real nested event
loop like Chromium's `base::RunLoop(kNestableTasksAllowed)`. The plugin thread
and main/UI thread are separate. When Flash calls `Run()`, it's on the
**plugin thread**, and it expects the system to remain responsive (processing
main-thread callbacks, timers, async completions) until `Quit()` is called.

**Strategy**: Keep the plugin thread blocked in `Run()` but **do NOT pump the
main-thread loop from the plugin thread**. Instead, rely on the main/UI thread's
own polling loop (`poll_main_loop()`) to keep processing callbacks. This is safe
because in ppapi-host the main thread is always running its own independent
polling loop.

The key fix is handling the case where `Quit()` is never called:

##### Changes to `flash_message_loop.rs`:

1. **Remove main-loop pumping from `run()`**: Stop draining/executing main-loop
   callbacks from the plugin thread. The main thread already does this.

2. **Add `run_called` guard**: Track whether `Run()` has been called to reject
   duplicate calls (return `PP_ERROR_FAILED`), matching Chrome.

3. **Add destruction-triggered quit**: When the resource is released (ref count
   → 0), signal the quit condvar with `PP_ERROR_ABORTED` so `Run()` unblocks.
   This requires implementing a `Drop` or a custom release hook.

4. **Add configurable timeout (safety net)**: If `Quit()` is never called AND
   the resource isn't destroyed, `Run()` should still eventually return. Add a
   generous timeout (e.g. 30 seconds) after which `Run()` returns
   `PP_ERROR_ABORTED`. Log a warning. This prevents infinite hangs in edge cases
   where neither Quit nor resource destruction occurs.

5. **Return value semantics**: Match Chrome's contract:
   - `PP_OK` if `Quit()` was called
   - `PP_ERROR_ABORTED` if resource destroyed or timeout
   - `PP_ERROR_FAILED` if `Run()` called more than once

##### Changes to `FlashMessageLoopResource`:

```rust
pub struct FlashMessageLoopResource {
    /// (quit_flag, result_code) protected by mutex + condvar
    state: Arc<(Mutex<FlashLoopState>, Condvar)>,
}

struct FlashLoopState {
    quit: bool,
    result: i32,      // PP_OK or PP_ERROR_ABORTED
    run_called: bool,
}
```

When the resource is released (ref_count → 0), its `Drop` impl sets
`quit = true, result = PP_ERROR_ABORTED` and notifies the condvar.

##### Pseudocode for `run()`:

```rust
fn run(flash_message_loop: PP_Resource) -> i32 {
    // 1. Extract Arc<state> from resource (short resource-lock)
    let state = get_state(flash_message_loop)?;

    // 2. Mark run_called; reject if already called
    {
        let mut s = state.0.lock();
        if s.run_called { return PP_ERROR_FAILED; }
        s.run_called = true;
    }

    // 3. Block until quit or timeout
    let (lock, cvar) = &*state;
    let mut guard = lock.lock();
    let timeout = Duration::from_secs(30);
    let start = Instant::now();

    while !guard.quit {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            tracing::warn!("PPB_Flash_MessageLoop::Run timed out after 30s");
            guard.result = PP_ERROR_ABORTED;
            break;
        }
        let (g, _timeout_result) = cvar.wait_timeout(guard, remaining).unwrap();
        guard = g;
    }

    guard.result
}
```

##### Pseudocode for `quit()`:

```rust
fn quit(flash_message_loop: PP_Resource) {
    let state = get_state(flash_message_loop);
    if let Some(state) = state {
        let (lock, cvar) = &*state;
        let mut s = lock.lock();
        s.quit = true;
        s.result = PP_OK;
        cvar.notify_one();
    }
}
```

##### Drop / release hook:

We need a mechanism for the resource to signal quit when destroyed. Options:

**Option A** — Implement `Drop` on `FlashMessageLoopResource`:
```rust
impl Drop for FlashMessageLoopResource {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.state;
        let mut s = lock.lock();
        if s.run_called && !s.quit {
            s.quit = true;
            s.result = PP_ERROR_ABORTED;
            cvar.notify_one();
        }
    }
}
```
This fires when `ResourceManager` removes the entry (ref count → 0).

**Option B** — Check if the resource is still alive in the `run()` loop by
periodically verifying it in the resource table. Less clean, more overhead.

**Recommendation**: Option A (Drop impl).

---

### 4B. PPB_MessageLoop Improvements

The current `PPB_MessageLoop` implementation is mostly correct for its use case
(background threads). The following improvements bring it closer to Chrome's
behavior:

#### 1. QuitWhenIdle semantics for `post_quit(should_destroy=true)`

Chrome's PostQuit with `should_destroy=true` drains remaining queued work before
the loop exits. Current implementation sends a null sentinel that causes
immediate break.

**Fix**: When `should_destroy` is true, after receiving the quit sentinel, drain
and execute all remaining items in the channel before returning.

```rust
// In run(), after breaking out of the main recv loop:
if self.destroyed && self.depth == 0 {
    // Execute all remaining queued callbacks with PP_ERROR_ABORTED
    // before final exit, so associated memory is freed.
    while let Ok(item) = self.receiver.try_recv() {
        if !item.callback.is_null() {
            unsafe { item.callback.run(PP_ERROR_ABORTED); }
        }
    }
    for item in self.deferred.drain(..) {
        if !item.callback.is_null() {
            unsafe { item.callback.run(PP_ERROR_ABORTED); }
        }
    }
}
```

#### 2. Nesting support (low priority)

Chrome supports nested `Run()` calls with a `nested_invocations_` counter.
The PPAPI spec says "you may not run nested message loops" for `PPB_MessageLoop`,
but Chrome does support it. Flash uses `PPB_Flash_MessageLoop` for nesting,
so this is low priority. Keep current behavior (reject nested calls) for now.

#### 3. PostQuit from non-current thread

Chrome handles PostQuit from a different thread by posting a closure to the
loop's task runner. Current implementation sends on the channel, which already
works cross-thread via crossbeam. ✓ Already correct.

---

## 5. Implementation Order

### Phase 1: Fix the freeze (immediate)

1. **Rewrite `flash_message_loop.rs`**:
   - New `FlashLoopState` struct with `quit`, `result`, `run_called` fields
   - `run()`: mark run_called, block on condvar (no main-loop pumping), timeout
   - `quit()`: set quit+result, notify condvar
   - `Drop` impl: signal PP_ERROR_ABORTED

2. **Test**: Load the SWF that triggers the ActionScript error. Verify:
   - System does NOT freeze
   - `Run()` returns (either via Quit() or timeout/destruction)
   - Main loop continues functioning (graphics, input, timers)

### Phase 2: PPB_MessageLoop improvements (incremental)

3. **Drain on destroy**: After quit sentinel in `run()`, drain remaining
   callbacks with `PP_ERROR_ABORTED` when `should_destroy=true`.

4. **Verify thread-safety**: Audit that `poll_main_loop` (main thread) and
   `PPB_MessageLoop::Run` (background thread) never race. Currently they operate
   on different MessageLoop instances (main vs background), so this is safe. ✓

### Phase 3: Full Flash_MessageLoop integration (future)

5. **Wire dialog completion**: When `PPB_Flash_Menu::Show` or dialog APIs are
   implemented, ensure their completion callbacks call
   `PPB_Flash_MessageLoop::Quit()` so `Run()` returns naturally.

6. **Consider reducing timeout**: Once dialogs work, the 30s safety-net timeout
   can be reduced or made configurable.

---

## 6. Files to Modify

| File | Changes |
|------|---------|
| `crates/ppapi-host/src/interfaces/flash_message_loop.rs` | Rewrite Run/Quit, add FlashLoopState, Drop impl, remove main-loop pumping |
| `crates/ppapi-host/src/message_loop.rs` | Add drain-on-destroy in `run()` |

No changes needed to:
- `player-core/src/lib.rs` (poll_main_loop is unaffected)
- `player-web/src/main.rs` (main loop is unaffected)
- `player-egui/src/app.rs` (update loop is unaffected)
- `interfaces/message_loop.rs` (vtable/wiring is fine)

---

## 7. Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Flash expects Run() to pump events and breaks if it doesn't | The 30s timeout ensures we don't hang forever; Flash continues (possibly with missing dialog) |
| Timeout too short — legitimate Quit() arrives after timeout | 30s is very generous; can increase if needed |
| Callback ordering changes | Main-loop callbacks now ONLY run on main thread (correct); no more wrong-thread execution |
| Drop not firing because Arc holds extra refs | FlashMessageLoopResource owns the Arc; cloned Arcs are only held by the run() stack frame, which releases when unblocked |
