package com.nodpi.android

import android.content.Context
import android.os.Process as OsProcess
import android.system.ErrnoException
import android.system.Os
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit

data class ServerActionResult(val success: Boolean, val message: String)

class ServerManager(
    private val context: Context,
    private val root: File,
    private val configStore: ConfigStore,
    execDir: File
) {
    private var process: Process? = null
    private val assetInstaller = AssetInstaller(context, root, execDir)
    private val pidStore = PidStore(root)

    fun isRunning(): Boolean = process?.isAlive == true

    fun start(config: ProxyConfig): ServerActionResult {
        if (isRunning()) {
            return ServerActionResult(true, "Server already running")
        }

        return try {
            cleanupStaleProcess()
            val nativeBinary = resolveNativeBinary()
            if (nativeBinary != null) {
                return startProcess(nativeBinary, config)
            }

            val assetBinary = assetInstaller.ensureServerBinary()
            startProcess(assetBinary, config)
        } catch (err: IOException) {
            if (err.message?.contains("Permission denied", ignoreCase = true) == true) {
                val nativeBinary = resolveNativeBinary()
                if (nativeBinary != null) {
                    return try {
                        startProcess(nativeBinary, config)
                    } catch (nested: IOException) {
                        ServerActionResult(false, "Start failed: ${nested.message}")
                    }
                }
                return ServerActionResult(
                    false,
                    "Start failed: Permission denied while executing from app data. " +
                        "The device may mount app data as noexec. " +
                        "Bundle the binary under app/src/main/jniLibs/<abi>/libnodpi_server.so so it is extracted " +
                        "to nativeLibraryDir, then reinstall the app."
                )
            }
            ServerActionResult(false, "Start failed: ${err.message}")
        }
    }

    fun stop(): ServerActionResult {
        val current = process
        if (current == null || !current.isAlive) {
            process = null
            pidStore.clear()
            return ServerActionResult(true, "Server stopped")
        }
        current.destroy()
        val stopped = current.waitFor(2, TimeUnit.SECONDS)
        if (!stopped) {
            current.destroyForcibly()
        }
        process = null
        pidStore.clear()
        return ServerActionResult(true, "Server stopped")
    }

    fun restart(config: ProxyConfig): ServerActionResult {
        stop()
        return start(config)
    }

    private fun startProcess(binary: File, config: ProxyConfig): ServerActionResult {
        ensureExecutable(binary)
        LogStore.append(
            "[info] exec: ${binary.absolutePath} exists=${binary.exists()} " +
                "canExec=${binary.canExecute()} canRead=${binary.canRead()} size=${binary.length()}"
        )
        val args = buildArgs(config)
        val processBuilder = ProcessBuilder(listOf(binary.absolutePath) + args)
            .directory(root)
        val started = processBuilder.start()
        process = started
        consume(started)
        recordPid(started)
        LogStore.append("[info] started: ${binary.absolutePath}")
        watchExit(started)
        return ServerActionResult(true, "Server started")
    }

    private fun resolveNativeBinary(): File? {
        val nativeDir = File(context.applicationInfo.nativeLibraryDir)
        val candidates = listOf(
            File(nativeDir, "libnodpi_server.so"),
            File(nativeDir, "nodpi_server.so"),
            File(nativeDir, "nodpi_server")
        )
        return candidates.firstOrNull { it.exists() }
    }

    private fun ensureExecutable(file: File) {
        file.setReadable(true, false)
        file.setWritable(true, true)
        file.setExecutable(true, false)
        try {
            Os.chmod(file.absolutePath, 0b111101101)
        } catch (_: ErrnoException) {
        } catch (_: SecurityException) {
        }
    }


    private fun buildArgs(config: ProxyConfig): List<String> {
        val args = mutableListOf(
            "--host", config.host,
            "--port", config.port.toString(),
            "--fragment-method", config.fragmentMethod,
            "--domain-matching", config.domainMatching
        )
        if (!config.noBlacklist && !config.autoBlacklist) {
            val blacklistPath = configStore.resolvePath(config.blacklistFile)
            args += listOf("--blacklist", blacklistPath.absolutePath)
        }
        config.outHost?.let { args += listOf("--out-host", it) }
        config.logAccessFile?.let {
            val path = configStore.resolvePath(it)
            args += listOf("--log-access", path.absolutePath)
        }
        config.logErrorFile?.let {
            val path = configStore.resolvePath(it)
            args += listOf("--log-error", path.absolutePath)
        }
        if (config.noBlacklist) args += "--no-blacklist"
        if (config.autoBlacklist) args += "--autoblacklist"
        if (config.quiet) args += "--quiet"
        return args
    }

    private fun consume(process: Process) {
        Thread {
            try {
                process.inputStream.bufferedReader().useLines { lines ->
                    lines.forEach { line ->
                        try {
                            LogStore.appendRaw(line)
                        } catch (_: Throwable) {
                        }
                    }
                }
            } catch (_: Throwable) {
            }
        }.start()
        Thread {
            try {
                process.errorStream.bufferedReader().useLines { lines ->
                    lines.forEach { line ->
                        try {
                            LogStore.appendRaw(line)
                        } catch (_: Throwable) {
                        }
                    }
                }
            } catch (_: Throwable) {
            }
        }.start()
    }

    private fun recordPid(process: Process) {
        val pid = readPid(process)
        if (pid != null) {
            pidStore.write(pid)
            LogStore.append("[info] pid: $pid")
        }
    }

    private fun readPid(process: Process): Long? {
        try {
            val method = process.javaClass.getMethod("pid")
            val value = method.invoke(process)
            return (value as? Long)
        } catch (_: Throwable) {
        }
        try {
            val field = process.javaClass.getDeclaredField("pid")
            field.isAccessible = true
            val value = field.get(process)
            return when (value) {
                is Int -> value.toLong()
                is Long -> value
                else -> null
            }
        } catch (_: Throwable) {
        }
        return null
    }

    private fun watchExit(process: Process) {
        Thread {
            val exitCode = try {
                process.waitFor()
            } catch (_: InterruptedException) {
                return@Thread
            }
            pidStore.clear()
            LogStore.append("[info] exited with code $exitCode")
        }.start()
    }

    private fun cleanupStaleProcess() {
        val pid = pidStore.read() ?: return
        if (pid <= 1 || pid == OsProcess.myPid().toLong()) {
            LogStore.append("[warn] pid file points to app pid: $pid")
            pidStore.clear()
            return
        }
        if (!isNodpiProcess(pid)) {
            LogStore.append("[warn] pid $pid is not nodpi_server; skipping kill")
            pidStore.clear()
            return
        }
        LogStore.append("[info] found pid file: $pid")
        killByPid(pid)
        pidStore.clear()
    }

    private fun killByPid(pid: Long? = null) {
        val target = pid ?: pidStore.read() ?: return
        if (target <= 1 || target == OsProcess.myPid().toLong()) {
            LogStore.append("[warn] refusing to kill app pid $target")
            return
        }
        try {
            ProcessBuilder("/system/bin/kill", "-9", target.toString()).start().waitFor()
            LogStore.append("[info] killed pid $target")
        } catch (err: Exception) {
            LogStore.append("[warn] kill failed for pid $target: ${err.message}")
        }
    }

    private fun isNodpiProcess(pid: Long): Boolean {
        return try {
            val cmdline = File("/proc/$pid/cmdline").readText()
            cmdline.contains("nodpi_server")
        } catch (_: Exception) {
            false
        }
    }
}
