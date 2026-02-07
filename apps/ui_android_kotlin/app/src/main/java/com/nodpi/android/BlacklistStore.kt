package com.nodpi.android

import java.io.File

class BlacklistStore(
    private val root: File,
    private val configStore: ConfigStore,
    private val assetInstaller: AssetInstaller
) {
    fun load(config: ProxyConfig): String {
        val file = resolve(config)
        assetInstaller.ensureDefaultBlacklist(file)
        if (file.exists()) {
            val text = file.readText()
            if (text.isNotBlank()) {
                return text
            }
        }
        return assetInstaller.loadDefaultBlacklist()
    }

    fun save(config: ProxyConfig, contents: String) {
        val file = resolve(config)
        file.parentFile?.mkdirs()
        file.writeText(contents)
    }

    private fun resolve(config: ProxyConfig): File {
        return configStore.resolvePath(config.blacklistFile)
    }
}
