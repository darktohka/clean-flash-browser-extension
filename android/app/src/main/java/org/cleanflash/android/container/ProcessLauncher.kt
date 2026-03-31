package org.cleanflash.android.container

import android.os.Process
import android.util.Log
import java.io.BufferedReader
import java.io.File
import java.io.FileWriter
import java.io.InputStreamReader
import java.util.concurrent.Executors

/**
 * Launches the flash-host process inside a PRoot chroot with Box64.
 *
 * Follows Winlator's GuestProgramLauncherComponent + ProcessHelper patterns.
 * PRoot is shipped as libproot.so (an ELF disguised as a shared library since
 * Android restricts execution of arbitrary binaries but allows loading native libs).
 */
class ProcessLauncher(
    private val nativeLibDir: String,
    private val rootfsDir: File,
    private val tmpDir: File,
    private val socketDir: File
) {
    companion object {
        private const val TAG = "ProcessLauncher"
    }

    private var pid: Int = -1
    private var process: java.lang.Process? = null
    private val lock = Any()

    /**
     * Launch the flash-host binary via PRoot + Box64.
     *
     * @param guestEnvVars Environment variables for the guest process
     * @param box64Available Whether Box64 is installed in the rootfs
     * @param extraBindPaths Additional paths to bind-mount into the chroot
     * @param onTermination Callback invoked when the process exits
     * @return The launched Process, or null on failure
     */
    fun launch(
        guestEnvVars: Map<String, String>,
        box64Available: Boolean = true,
        extraBindPaths: List<String> = emptyList(),
        onTermination: ((exitCode: Int) -> Unit)? = null
    ): java.lang.Process? {
        synchronized(lock) {
            stop()

            val prootPath = findProot()
            if (prootPath == null) {
                Log.e(TAG, "PRoot not found")
                return null
            }
            val command = buildProotCommand(prootPath, box64Available, extraBindPaths)

            Log.i(TAG, "Launch command: ${command.joinToString(" ")}")

            return try {
                val hostEnv = EnvironmentSetup.buildProotHostEnvVars(nativeLibDir, tmpDir.absolutePath)

                val pb = ProcessBuilder(command)
                pb.directory(rootfsDir)
                pb.environment().clear()
                for (e in hostEnv) {
                    val parts = e.split("=", limit = 2)
                    if (parts.size == 2) {
                        pb.environment()[parts[0]] = parts[1]
                    }
                }
                // Guest env vars inherited through PRoot to the guest process
                for ((key, value) in guestEnvVars) {
                    pb.environment()[key] = value
                }

                // Merge stderr into stdout so we can capture Box64/flash-host logs
                pb.redirectErrorStream(true)

                // Ensure output log file exists and starts fresh each launch.
                val hostLogFile = File(tmpDir, "flash-host-stderr.log")
                hostLogFile.parentFile?.mkdirs()
                if (hostLogFile.exists()) {
                    hostLogFile.delete()
                }
                hostLogFile.createNewFile()

                val proc = pb.start()
                process = proc
                pid = getPid(proc)

                Log.i(TAG, "Host process started, PID=$pid")

                // Stream merged host output to both logcat and a persistent file.
                createDebugReader(proc, hostLogFile)

                // Wait-for-exit thread
                if (onTermination != null) {
                    createWaitThread(proc, onTermination)
                }

                proc
            } catch (e: Exception) {
                Log.e(TAG, "Failed to launch host process", e)
                null
            }
        }
    }

    /**
     * Stop the running host process.
     */
    fun stop() {
        synchronized(lock) {
            if (pid != -1) {
                Log.i(TAG, "Killing host process PID=$pid")
                try {
                    Process.killProcess(pid)
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to kill process $pid", e)
                }
                pid = -1
            }
            process?.destroy()
            process = null
        }
    }

    /**
     * Suspend the host process (SIGSTOP).
     */
    fun suspend() {
        synchronized(lock) {
            if (pid != -1) {
                Process.sendSignal(pid, 19) // SIGSTOP
            }
        }
    }

    /**
     * Resume a suspended host process (SIGCONT).
     */
    fun resume() {
        synchronized(lock) {
            if (pid != -1) {
                Process.sendSignal(pid, 18) // SIGCONT
            }
        }
    }

    val isRunning: Boolean get() = synchronized(lock) { pid != -1 }

    // ---- Private ----

    private fun findProot(): String? {
        val prootPath = "$nativeLibDir/libproot.so"
        if (File(prootPath).exists()) return prootPath
        return null
    }

    /**
     * Build the PRoot command line.
     *
     * Guest environment variables are passed through the process environment
     * (inherited by PRoot and then by the guest process).
     */
    private fun buildProotCommand(
        prootPath: String,
        box64Available: Boolean,
        extraBindPaths: List<String>
    ): List<String> {
        val cmd = mutableListOf<String>()

        cmd.add(prootPath)
        cmd.add("--kill-on-exit")
        cmd.add("--rootfs=${rootfsDir.absolutePath}")
        cmd.add("--cwd=/home/flash")

        // Standard bind mounts
        cmd.add("--bind=/dev")
        cmd.add("--bind=/proc")
        cmd.add("--bind=/sys")

        // Bind the socket directory
        cmd.add("--bind=${socketDir.absolutePath}:${socketDir.absolutePath}")

        for (path in extraBindPaths) {
            val file = File(path)
            if (file.exists()) {
                cmd.add("--bind=${file.absolutePath}")
            }
        }

        // Box64 wraps flash-host for x86_64 translation
        if (box64Available) {
            cmd.add("/usr/local/bin/box64")
        }

        cmd.add("/opt/flash/flash-host")
        return cmd
    }

    /**
     * Extract PID from a Process using reflection.
     * Following Winlator's ProcessHelper.exec() pattern.
     */
    private fun getPid(process: java.lang.Process): Int {
        return try {
            val pidField = process.javaClass.getDeclaredField("pid")
            pidField.isAccessible = true
            val p = pidField.getInt(process)
            pidField.isAccessible = false
            p
        } catch (e: Exception) {
            Log.w(TAG, "Could not extract PID via reflection", e)
            -1
        }
    }

    private fun createDebugReader(process: java.lang.Process, logFile: File) {
        Executors.newSingleThreadExecutor().execute {
            try {
                FileWriter(logFile, true).use { writer ->
                    BufferedReader(InputStreamReader(process.inputStream)).use { reader ->
                        var line: String?
                        while (reader.readLine().also { line = it } != null) {
                            val current = line ?: continue
                            Log.d(TAG, "host: $current")
                            writer.append(current).append('\n')
                            writer.flush()
                        }
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to capture host output", e)
            }
        }
    }

    private fun createWaitThread(
        process: java.lang.Process,
        onTermination: (Int) -> Unit
    ) {
        Executors.newSingleThreadExecutor().execute {
            try {
                val exitCode = process.waitFor()
                synchronized(lock) {
                    pid = -1
                    this.process = null
                }
                Log.i(TAG, "Host process exited with code $exitCode")
                onTermination(exitCode)
            } catch (_: InterruptedException) { }
        }
    }
}
