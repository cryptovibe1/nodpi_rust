package com.nodpi.android

import android.content.Context
import android.os.Build
import android.system.ErrnoException
import android.system.Os
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

class AssetInstaller(
    private val context: Context,
    private val root: File,
    private val execDir: File
) {
    private val binDir = execDir
    private val assetRoot = "servers"

    fun ensureServerBinary(): File {
        val target = File(binDir, "nodpi_server")
        if (target.exists()) {
            return target
        }
        binDir.mkdirs()

        val abi = selectAbiFolder()
            ?: throw IOException(
                "Unsupported ABI. Supported on device: ${Build.SUPPORTED_ABIS.joinToString(", ")}. " +
                    "Expected assets under $assetRoot/<abi>/nodpi_server"
            )
        val assetPath = "$assetRoot/$abi/nodpi_server"
        if (!assetExists(assetPath)) {
            throw IOException(
                "Missing server binary asset. Tried: $assetPath. " +
                    "Expected assets under assets/$assetRoot/<abi>/nodpi_server for ABI '$abi'. " +
                    "Device ABIs: ${Build.SUPPORTED_ABIS.joinToString(", ")}."
            )
        }
        copyAsset(assetPath, target)
        applyExecutablePermissions(target)
        return target
    }

    fun ensureDefaultBlacklist(target: File) {
        if (target.exists()) return
        val assetPath = "blacklist.txt"
        if (!assetExists(assetPath)) return
        target.parentFile?.mkdirs()
        copyAsset(assetPath, target)
    }

    fun loadDefaultBlacklist(): String {
        val assetPath = "blacklist.txt"
        return try {
            context.assets.open(assetPath).bufferedReader().use { it.readText() }
        } catch (_: IOException) {
            ""
        }
    }

    private fun selectAbiFolder(): String? {
        val supported = Build.SUPPORTED_ABIS
        return supported.firstNotNullOfOrNull { abiToFolder(it) }
    }

    private fun abiToFolder(abi: String): String? {
        return when (abi) {
            "arm64-v8a" -> "arm64-v8a"
            "armeabi-v7a" -> "armeabi-v7a"
            "x86" -> "x86"
            "x86_64" -> "x86_64"
            else -> null
        }
    }

    private fun assetExists(path: String): Boolean {
        return try {
            context.assets.open(path).close()
            true
        } catch (_: IOException) {
            false
        }
    }

    private fun copyAsset(path: String, target: File) {
        context.assets.open(path).use { input ->
            FileOutputStream(target).use { output ->
                input.copyTo(output)
            }
        }
    }

    private fun applyExecutablePermissions(target: File) {
        target.setReadable(true, false)
        target.setWritable(true, true)
        target.setExecutable(true, false)
        try {
            Os.chmod(target.absolutePath, 0b111101101)
        } catch (_: ErrnoException) {
        } catch (_: SecurityException) {
        }
    }
}
