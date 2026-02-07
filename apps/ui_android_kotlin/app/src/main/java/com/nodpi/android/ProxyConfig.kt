package com.nodpi.android

data class ProxyConfig(
    var host: String = "0.0.0.0",
    var port: Int = 8881,
    var blacklistFile: String = "blacklist.txt",
    var fragmentMethod: String = "random",
    var domainMatching: String = "strict",
    var outHost: String? = null,
    var logAccessFile: String? = null,
    var logErrorFile: String? = null,
    var noBlacklist: Boolean = false,
    var autoBlacklist: Boolean = false,
    var quiet: Boolean = false
)
