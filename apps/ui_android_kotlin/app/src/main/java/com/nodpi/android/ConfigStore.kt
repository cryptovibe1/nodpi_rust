package com.nodpi.android

import java.io.File

class ConfigStore(private val root: File) {
    private val configFile = File(File(root, "config"), "nodpi.conf")

    fun load(): ProxyConfig {
        if (!configFile.exists()) {
            val cfg = ProxyConfig()
            save(cfg)
            return cfg
        }
        val text = configFile.readText()
        return parse(text)
    }

    fun save(config: ProxyConfig) {
        configFile.parentFile?.mkdirs()
        configFile.writeText(serialize(config))
    }

    fun resolvePath(path: String): File {
        val file = File(path)
        return if (file.isAbsolute) file else File(root, path)
    }

    private fun parse(text: String): ProxyConfig {
        val cfg = ProxyConfig()
        text.lines().forEach { line ->
            val trimmed = line.trim()
            if (trimmed.isEmpty() || trimmed.startsWith("#")) return@forEach
            val parts = trimmed.split("=", limit = 2)
            if (parts.size != 2) return@forEach
            val key = parts[0].trim()
            val value = parts[1].trim()
            when (key) {
                "HOST" -> cfg.host = value
                "PORT" -> cfg.port = value.toIntOrNull() ?: cfg.port
                "BLACKLIST_FILE" -> cfg.blacklistFile = value
                "FRAGMENT_METHOD" -> cfg.fragmentMethod = value
                "DOMAIN_MATCHING" -> cfg.domainMatching = value
                "OUT_HOST" -> cfg.outHost = value.ifBlank { null }
                "LOG_ACCESS_FILE" -> cfg.logAccessFile = value.ifBlank { null }
                "LOG_ERROR_FILE" -> cfg.logErrorFile = value.ifBlank { null }
                "NO_BLACKLIST" -> cfg.noBlacklist = value.equals("true", ignoreCase = true)
                "AUTO_BLACKLIST" -> cfg.autoBlacklist = value.equals("true", ignoreCase = true)
                "QUIET" -> cfg.quiet = value.equals("true", ignoreCase = true)
            }
        }
        return cfg
    }

    private fun serialize(cfg: ProxyConfig): String {
        return buildString {
            append("HOST=").append(cfg.host).append('\n')
            append("PORT=").append(cfg.port).append('\n')
            append("BLACKLIST_FILE=").append(cfg.blacklistFile).append('\n')
            append("FRAGMENT_METHOD=").append(cfg.fragmentMethod).append('\n')
            append("DOMAIN_MATCHING=").append(cfg.domainMatching).append('\n')
            append("OUT_HOST=").append(cfg.outHost.orEmpty()).append('\n')
            append("LOG_ACCESS_FILE=").append(cfg.logAccessFile.orEmpty()).append('\n')
            append("LOG_ERROR_FILE=").append(cfg.logErrorFile.orEmpty()).append('\n')
            append("NO_BLACKLIST=").append(cfg.noBlacklist).append('\n')
            append("AUTO_BLACKLIST=").append(cfg.autoBlacklist).append('\n')
            append("QUIET=").append(cfg.quiet).append('\n')
        }
    }
}
