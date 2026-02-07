package com.nodpi.android

import java.io.File

class PidStore(root: File) {
    private val pidFile = File(File(root, "var"), "nodpi.pid")

    fun read(): Long? {
        return try {
            pidFile.readText().trim().toLongOrNull()
        } catch (_: Exception) {
            null
        }
    }

    fun write(pid: Long) {
        pidFile.parentFile?.mkdirs()
        pidFile.writeText(pid.toString())
    }

    fun clear() {
        if (pidFile.exists()) {
            pidFile.delete()
        }
    }
}
