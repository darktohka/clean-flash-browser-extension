package org.cleanflash.android

import android.content.Context
import android.content.SharedPreferences
import android.net.Uri
import android.util.Log
import org.cleanflash.android.container.EnvironmentSetup
import org.cleanflash.android.container.ProcessLauncher
import org.cleanflash.android.container.TarExtractor
import java.io.File
import java.io.FileOutputStream

/**
 * Manages the Ubuntu chroot environment for running flash-host + Box64.
 *
 * Follows Winlator's ImageFs + ImageFsInstaller + GuestProgramLauncherComponent
 * patterns, adapted for our simpler Flash-only use case.
 *
 * Handles:
 * - First-run rootfs extraction from APK assets (rootfs.tar.zst)
 * - Version-gated re-extraction on app updates
 * - Box64 versioned extraction and upgrade
 * - flash-host binary installation into the chroot
 * - PRoot + Box64 process launching via ProcessLauncher
 * - File management within the chroot (SWF copying, temp files)
 */
class ContainerManager(private val context: Context) {

    companion object {
        private const val TAG = "ContainerManager"

        /** Increment this when the rootfs archive changes. */
        private const val ROOTFS_VERSION = 2

        /** Default Box64 version to use. */
        const val DEFAULT_BOX64_VERSION = "0.2.8"

        // Asset file names
        private const val ROOTFS_ASSET = "rootfs.tar.zst"
        private const val BOX64_ASSET_PREFIX = "box64-v"
        private const val BOX64_ASSET_SUFFIX = ".tar.zst"

        // SharedPreferences keys
        private const val PREFS_NAME = "container_versions"
        private const val KEY_ROOTFS_VERSION = "rootfs_version"
        private const val KEY_BOX64_VERSION = "current_box64_version"
        private const val KEY_HOST_VERSION = "current_host_version"
    }

    // ---- Directory layout (following architecture doc §5.1) ----

    val rootfsDir: File get() = File(context.filesDir, "rootfs")

    /** PRoot temp directory (outside rootfs). */
    val prootTmpDir: File get() = File(context.filesDir, "tmp")

    /** IPC socket directory (bind-mounted into chroot). */
    val socketDir: File get() = File(context.filesDir, "sockets")

    /** Per-session temp files inside the chroot. */
    val flashTmpDir: File get() = File(rootfsDir, "tmp/flash")

    /** Flash host binary location inside rootfs. */
    val hostBinaryPath: File get() = File(rootfsDir, "opt/flash/flash-host")

    /** Flash plugin location inside rootfs. */
    val pluginPath: File get() = File(rootfsDir, "opt/flash/libpepflashplayer.so")

    /** Box64 binary location inside rootfs. */
    val box64Path: File get() = File(rootfsDir, "usr/local/bin/box64")

    /** User home directory inside rootfs. */
    val homeDir: File get() = File(rootfsDir, "home/flash")

    private val prefs: SharedPreferences
        get() = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    private var processLauncher: ProcessLauncher? = null

    // ---- Initialization state ----

    /** Whether the rootfs has been extracted at the expected version. */
    val isRootfsReady: Boolean
        get() = prefs.getInt(KEY_ROOTFS_VERSION, 0) >= ROOTFS_VERSION

    /** Whether Box64 is available in the rootfs. */
    val isBox64Ready: Boolean
        get() = box64Path.exists()

    /** Whether the flash-host binary is installed. */
    val isHostReady: Boolean
        get() = hostBinaryPath.exists() && hostBinaryPath.canExecute()

    /** Full initialization check. */
    val isInitialized: Boolean
        get() = isRootfsReady && isHostReady

    // ---- Public API ----

    /**
     * Initialize the container environment. Idempotent — skips steps that are
     * already at the correct version.
     *
     * @param onProgress Optional progress callback: (stage, detail)
     */
    fun initialize(onProgress: ((String, String) -> Unit)? = null) {
        ensureDirectories()
        installRootfs(onProgress)
        installBox64(onProgress)
        installHostBinary(onProgress)
    }

    /**
     * Get the absolute socket path for a named socket.
     * The socket directory is bind-mounted into the chroot so the path
     * is accessible from both inside and outside.
     */
    fun getSocketPath(name: String): String {
        socketDir.mkdirs()
        // Clean up any stale socket from a previous run
        val socketFile = File(socketDir, name)
        if (socketFile.exists()) socketFile.delete()
        return socketFile.absolutePath
    }

    /**
     * Copy a content URI file into the chroot temp directory for loading.
     * @return Absolute path to the copied file (accessible from inside chroot via bind mount)
     */
    fun copyFileToChroot(uri: Uri): String {
        flashTmpDir.mkdirs()
        val fileName = "input_${System.currentTimeMillis()}.swf"
        val destFile = File(flashTmpDir, fileName)

        context.contentResolver.openInputStream(uri)?.use { input ->
            FileOutputStream(destFile).use { output ->
                input.copyTo(output)
            }
        } ?: throw IllegalArgumentException("Cannot open URI: $uri")

        return "/tmp/flash/$fileName"
    }

    /**
     * Copy a raw file (e.g., libpepflashplayer.so) into the chroot.
     * @param source Source file on the device
     * @param chrootRelPath Destination path relative to rootfs (e.g., "opt/flash/libpepflashplayer.so")
     */
    fun installFileToChroot(source: File, chrootRelPath: String) {
        val dest = File(rootfsDir, chrootRelPath)
        dest.parentFile?.mkdirs()
        source.copyTo(dest, overwrite = true)
        dest.setExecutable(true, false)
        dest.setReadable(true, false)
        Log.i(TAG, "Installed ${source.name} → $chrootRelPath")
    }

    /**
     * Launch the flash-host process via PRoot + Box64.
     *
     * @param socketPath Absolute path to the IPC control socket
     * @param swfUrl URL or file:// path to the SWF
     * @param width SWF display width
     * @param height SWF display height
     * @param box64Preset Box64 dynarec tuning preset
     * @param enableLogs Enable Box64/host debug logging
     * @param onTermination Callback when the process exits
     * @return The Process, or null on failure
     */
    fun launchHost(
        socketPath: String,
        swfUrl: String,
        width: Int,
        height: Int,
        box64Preset: EnvironmentSetup.Box64Preset = EnvironmentSetup.Box64Preset.COMPATIBILITY,
        enableLogs: Boolean = false,
        onTermination: ((Int) -> Unit)? = null
    ): Process? {
        val nativeLibDir = context.applicationInfo.nativeLibraryDir

        val launcher = ProcessLauncher(
            nativeLibDir = nativeLibDir,
            rootfsDir = rootfsDir,
            tmpDir = prootTmpDir,
            socketDir = socketDir
        )
        processLauncher = launcher

        val guestEnv = EnvironmentSetup.buildGuestEnvVars(
            socketPath = socketPath,
            swfUrl = swfUrl,
            width = width,
            height = height,
            preset = box64Preset,
            enableLogs = enableLogs
        )

        return launcher.launch(
            guestEnvVars = guestEnv,
            box64Available = isBox64Ready,
            onTermination = onTermination
        )
    }

    /**
     * Stop the running host process.
     */
    fun stopHost() {
        processLauncher?.stop()
    }

    /**
     * Clean up temp files from previous sessions.
     */
    fun cleanTempFiles() {
        flashTmpDir.listFiles()?.forEach { file ->
            if (file.isFile) file.delete()
        }
    }

    /**
     * Get the host process stderr log content (for debugging).
     */
    fun getHostLog(): String? {
        val logFile = File(prootTmpDir, "flash-host-stderr.log")
        return if (logFile.exists()) logFile.readText() else null
    }

    // ---- Private: Rootfs installation ----

    private fun ensureDirectories() {
        rootfsDir.mkdirs()
        prootTmpDir.mkdirs()
        socketDir.mkdirs()
        flashTmpDir.mkdirs()

        // Ensure standard chroot directory structure exists
        for (dir in listOf(
            "bin", "etc", "lib", "lib64",
            "usr/bin", "usr/lib",
            "lib/x86_64-linux-gnu",
            "opt/flash",
            "home/flash/.flash/storage",
            "tmp/flash",
            "dev", "proc", "sys"
        )) {
            File(rootfsDir, dir).mkdirs()
        }
    }

    /**
     * Extract the rootfs archive from APK assets if needed.
     * Following Winlator's ImageFsInstaller.installIfNeeded() pattern:
     * - Check version, skip if current
     * - Clear old rootfs (preserving user data in /home/flash)
     * - Extract new rootfs
     * - Write version marker
     */
    private fun installRootfs(onProgress: ((String, String) -> Unit)?) {
        val currentVersion = prefs.getInt(KEY_ROOTFS_VERSION, 0)
        if (currentVersion >= ROOTFS_VERSION) {
            Log.i(TAG, "Rootfs already at version $currentVersion (need $ROOTFS_VERSION)")
            return
        }

        onProgress?.invoke("rootfs", "Extracting system files...")
        Log.i(TAG, "Installing rootfs: $currentVersion → $ROOTFS_VERSION")

        // Check if the asset exists
        val assetNames = try { context.assets.list("") ?: emptyArray() } catch (_: Exception) { emptyArray() }

        if (assetNames.contains(ROOTFS_ASSET)) {
            // Clear old rootfs files, preserving /home/flash (user data)
            clearRootfs()

            // Extract the zstd-compressed tar archive
            val success = TarExtractor.extractFromAssets(
                context = context,
                assetName = ROOTFS_ASSET,
                destination = rootfsDir,
                compression = TarExtractor.Compression.ZSTD
            ) { count, name ->
                if (count % 50 == 0) {
                    onProgress?.invoke("rootfs", "Extracting: $name ($count files)")
                }
            }

            if (!success) {
                Log.e(TAG, "Failed to extract rootfs")
                return
            }
        } else if (assetNames.contains("rootfs.tar")) {
            // Fallback: uncompressed tar
            clearRootfs()
            TarExtractor.extractFromAssets(
                context = context,
                assetName = "rootfs.tar",
                destination = rootfsDir,
                compression = TarExtractor.Compression.NONE
            )
        } else {
            Log.w(TAG, "No rootfs archive found in assets, creating minimal structure")
            // Create minimal /etc files
            writeMinimalEtc()
        }

        // Re-create directories that must exist
        ensureDirectories()

        // Write version marker
        prefs.edit().putInt(KEY_ROOTFS_VERSION, ROOTFS_VERSION).apply()
        Log.i(TAG, "Rootfs installed (version $ROOTFS_VERSION)")
    }

    /**
     * Clear the rootfs while preserving user data.
     * Following Winlator's clearRootDir() pattern.
     */
    private fun clearRootfs() {
        val preserveDirs = setOf("home", "tmp")
        rootfsDir.listFiles()?.forEach { file ->
            if (file.name !in preserveDirs) {
                if (file.isDirectory) file.deleteRecursively()
                else file.delete()
            }
        }
    }

    private fun writeMinimalEtc() {
        val etcDir = File(rootfsDir, "etc")
        etcDir.mkdirs()

        File(etcDir, "passwd").writeText(
            "root:x:0:0:root:/root:/bin/sh\nflash:x:1000:1000:Flash:/home/flash:/bin/sh\n"
        )
        File(etcDir, "group").writeText(
            "root:x:0:\nflash:x:1000:\n"
        )
        File(etcDir, "resolv.conf").writeText("nameserver 8.8.8.8\n")
        File(etcDir, "hosts").writeText("127.0.0.1 localhost\n")
        File(etcDir, "nsswitch.conf").writeText(
            "passwd: files\ngroup: files\nhosts: files dns\n"
        )
    }

    // ---- Private: Box64 installation ----

    /**
     * Extract Box64 from APK assets if needed.
     * Following Winlator's extractBox86_64Files() pattern:
     * versioned archives compared against SharedPreferences.
     */
    private fun installBox64(onProgress: ((String, String) -> Unit)?) {
        val desiredVersion = DEFAULT_BOX64_VERSION
        val currentVersion = prefs.getString(KEY_BOX64_VERSION, "") ?: ""

        if (currentVersion == desiredVersion && box64Path.exists()) {
            Log.i(TAG, "Box64 already at version $currentVersion")
            return
        }

        val assetName = "$BOX64_ASSET_PREFIX$desiredVersion$BOX64_ASSET_SUFFIX"
        val assetNames = try { context.assets.list("") ?: emptyArray() } catch (_: Exception) { emptyArray() }

        if (assetNames.contains(assetName)) {
            onProgress?.invoke("box64", "Installing Box64 v$desiredVersion...")
            Log.i(TAG, "Extracting Box64 $assetName")

            val success = TarExtractor.extractFromAssets(
                context = context,
                assetName = assetName,
                destination = rootfsDir,
                compression = TarExtractor.Compression.ZSTD
            )

            if (success) {
                box64Path.setExecutable(true, false)
                prefs.edit().putString(KEY_BOX64_VERSION, desiredVersion).apply()
                Log.i(TAG, "Box64 v$desiredVersion installed")
            } else {
                Log.e(TAG, "Failed to extract Box64")
            }
        } else {
            Log.w(TAG, "Box64 asset not found: $assetName")
        }
    }

    // ---- Private: Host binary installation ----

    /**
     * Copy the flash-host binary from APK native libs to the chroot.
     *
     * The binary is shipped as "libflash-host.so" in jniLibs/arm64-v8a/
     * (disguised as a shared library for Android's lib extraction).
     */
    private fun installHostBinary(onProgress: ((String, String) -> Unit)?) {
        val nativeLibDir = context.applicationInfo.nativeLibraryDir
        val srcFile = File(nativeLibDir, "libflash-host.so")

        if (!srcFile.exists()) {
            Log.w(TAG, "flash-host binary not found in native libs: ${srcFile.absolutePath}")
            return
        }

        // Check if already the same version (compare file size as quick check)
        val appVersion = try {
            context.packageManager.getPackageInfo(context.packageName, 0).versionCode.toString()
        } catch (_: Exception) { "0" }

        val currentHostVersion = prefs.getString(KEY_HOST_VERSION, "") ?: ""
        if (currentHostVersion == appVersion && hostBinaryPath.exists()) {
            Log.i(TAG, "flash-host already at app version $appVersion")
            return
        }

        onProgress?.invoke("host", "Installing flash-host binary...")
        hostBinaryPath.parentFile?.mkdirs()
        srcFile.copyTo(hostBinaryPath, overwrite = true)
        hostBinaryPath.setExecutable(true, false)
        hostBinaryPath.setReadable(true, false)

        prefs.edit().putString(KEY_HOST_VERSION, appVersion).apply()
        Log.i(TAG, "flash-host installed (app version $appVersion)")
    }
}
