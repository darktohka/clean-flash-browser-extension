package org.cleanflash.android.container

/**
 * Configures environment variables for the PRoot + Box64 guest environment.
 *
 * Follows Winlator's GuestProgramLauncherComponent pattern for env var setup.
 */
class EnvironmentSetup {

    /**
     * Box64 dynarec preset profiles, following Winlator's Box86_64PresetManager.
     */
    enum class Box64Preset(
        val safeFlags: Int,
        val fastNan: Int,
        val bigBlock: Int,
        val strongMem: Int
    ) {
        STABILITY(safeFlags = 2, fastNan = 0, bigBlock = 0, strongMem = 2),
        COMPATIBILITY(safeFlags = 2, fastNan = 0, bigBlock = 0, strongMem = 1),
        INTERMEDIATE(safeFlags = 2, fastNan = 1, bigBlock = 1, strongMem = 0),
        PERFORMANCE(safeFlags = 1, fastNan = 1, bigBlock = 3, strongMem = 0);
    }

    companion object {
        /**
         * Build the complete set of environment variables for the guest process.
         *
         * These are set inside the PRoot chroot via /usr/bin/env.
         *
         * @param socketPath Absolute path to the IPC control socket
         * @param swfUrl URL or file:// path to the SWF
         * @param width SWF display width
         * @param height SWF display height
         * @param preset Box64 dynarec tuning preset
         * @param enableLogs Whether to enable Box64 debug logging
         * @return Map of environment variable name to value
         */
        fun buildGuestEnvVars(
            socketPath: String,
            swfUrl: String,
            width: Int,
            height: Int,
            preset: Box64Preset = Box64Preset.COMPATIBILITY,
            enableLogs: Boolean = false
        ): Map<String, String> {
            val env = LinkedHashMap<String, String>()

            // ---- Core environment ----
            env["HOME"] = "/home/flash"
            env["USER"] = "flash"
            env["TMPDIR"] = "/tmp"
            env["LC_ALL"] = "C"
            env["PATH"] = "/usr/local/bin:/usr/bin:/bin"
            env["LD_LIBRARY_PATH"] = "/lib/aarch64-linux-gnu:/usr/lib/aarch64-linux-gnu"

            // ---- Flash host configuration ----
            env["FLASH_IPC_SOCKET"] = socketPath
            env["FLASH_SWF_URL"] = swfUrl
            env["FLASH_SWF_WIDTH"] = width.toString()
            env["FLASH_SWF_HEIGHT"] = height.toString()
            env["FLASH_LOG_DIR"] = "/tmp/flash"
            env["FLASH_PLUGIN_PATH"] = "/opt/flash/libpepflashplayer.so"

            // ---- Box64 configuration ----
            env["BOX64_DYNAREC"] = "1"
            env["BOX64_AVX"] = "1"
            env["BOX64_NOBANNER"] = if (enableLogs) "0" else "1"
            env["BOX64_LOG"] = if (enableLogs) "1" else "0"
            env["BOX64_NORCFILES"] = "1"
            env["BOX64_LD_LIBRARY_PATH"] = "/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu"

            // Box64 dynarec tuning (from preset)
            env["BOX64_DYNAREC_SAFEFLAGS"] = preset.safeFlags.toString()
            env["BOX64_DYNAREC_FASTNAN"] = preset.fastNan.toString()
            env["BOX64_DYNAREC_BIGBLOCK"] = preset.bigBlock.toString()
            env["BOX64_DYNAREC_STRONGMEM"] = preset.strongMem.toString()

            if (enableLogs) {
                env["BOX64_DYNAREC_MISSING"] = "1"
            }

            return env
        }

        /**
         * Build host-side environment variables for PRoot itself.
         *
         * These are set OUTSIDE the chroot as process environment vars.
         * Following Winlator's pattern for PROOT_TMP_DIR, PROOT_LOADER, etc.
         *
         * @param nativeLibDir Path to the app's native library directory
         * @param tmpDir Writable temp directory for PRoot
         * @return Array of "KEY=VALUE" strings for ProcessBuilder
         */
        fun buildProotHostEnvVars(
            nativeLibDir: String,
            tmpDir: String
        ): Array<String> {
            val env = mutableListOf<String>()

            env.add("PROOT_TMP_DIR=$tmpDir")
            env.add("PROOT_LOADER=$nativeLibDir/libproot-loader.so")

            // PROOT_LOADER_32 for 32-bit support (optional)
            val loader32 = "$nativeLibDir/libproot-loader32.so"
            if (java.io.File(loader32).exists()) {
                env.add("PROOT_LOADER_32=$loader32")
            }

            return env.toTypedArray()
        }

        /**
         * Format environment variables for /usr/bin/env command line.
         * Escapes values containing spaces or special characters.
         */
        fun formatForEnvCommand(envVars: Map<String, String>): String {
            return envVars.entries.joinToString(" ") { (key, value) ->
                if (value.contains(' ') || value.contains('"') ||
                    value.contains('\'') || value.contains('$')) {
                    "$key=\"${value.replace("\"", "\\\"")}\""
                } else {
                    "$key=$value"
                }
            }
        }
    }
}
