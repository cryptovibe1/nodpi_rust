package com.nodpi.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.app.PendingIntent
import android.net.TrafficStats
import android.os.Process
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import java.io.File

class ServerService : Service() {
    companion object {
        private const val channelId = "nodpi_server_v2"
        private const val channelName = "NoDPI Server"
        private const val notificationId = 1001
        private const val actionStart = "com.nodpi.android.action.START"
        private const val actionStop = "com.nodpi.android.action.STOP"
        private const val actionRestart = "com.nodpi.android.action.RESTART"

        @Volatile
        private var running: Boolean = false

        fun start(context: Context) {
            val intent = Intent(context, ServerService::class.java).setAction(actionStart)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            val intent = Intent(context, ServerService::class.java).setAction(actionStop)
            context.startService(intent)
        }

        fun restart(context: Context) {
            val intent = Intent(context, ServerService::class.java).setAction(actionRestart)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun isRunning(): Boolean = running
    }

    private lateinit var serverManager: ServerManager
    private lateinit var configStore: ConfigStore
    @Volatile
    private var monitorActive = false
    private val monitorLock = Any()
    private var lastRx: Long = -1
    private var lastTx: Long = -1
    private var lastTimeMs: Long = 0
    @Volatile
    private var trafficSummary: String = "Traffic: n/a"

    override fun onCreate() {
        super.onCreate()
        val rootDir = File(filesDir, "nodpi")
        val execDir = File(codeCacheDir, "nodpi_bin")
        configStore = ConfigStore(rootDir)
        serverManager = ServerManager(this, rootDir, configStore, execDir)
        ensureChannel()
        startForeground(notificationId, buildNotification("Server stopped", false, false, configStore.load()))
        ensureMonitor()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            actionStart -> {
                startForeground(notificationId, buildNotification("Starting server", false, true, configStore.load()))
                Thread {
                    val config = configStore.load()
                    val result = serverManager.start(config)
                    LogStore.append("[info] service start: ${result.message}")
                    running = result.success && serverManager.isRunning()
                    updateNotification(if (running) "Server running" else "Start failed", running, false, config)
                }.start()
            }
            actionStop -> {
                Thread {
                    val result = serverManager.stop()
                    LogStore.append("[info] service stop: ${result.message}")
                    running = false
                    updateNotification("Server stopped", false, false, configStore.load())
                }.start()
            }
            actionRestart -> {
                Thread {
                    val config = configStore.load()
                    updateNotification("Restarting server", running, true, config)
                    serverManager.stop()
                    val result = serverManager.start(config)
                    LogStore.append("[info] service restart: ${result.message}")
                    running = result.success && serverManager.isRunning()
                    updateNotification(if (running) "Server running" else "Restart failed", running, false, config)
                }.start()
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        monitorActive = false
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(channelId, channelName, NotificationManager.IMPORTANCE_DEFAULT)
        channel.description = "Server control panel"
        manager.createNotificationChannel(channel)
    }

    private fun updateNotification(message: String, isRunning: Boolean, showProgress: Boolean, config: ProxyConfig) {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(notificationId, buildNotification(message, isRunning, showProgress, config))
    }

    private fun buildNotification(message: String, isRunning: Boolean, showProgress: Boolean, config: ProxyConfig): Notification {
        val openIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val stopIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, ServerService::class.java).setAction(actionStop),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val startIntent = PendingIntent.getService(
            this,
            2,
            Intent(this, ServerService::class.java).setAction(actionStart),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val restartIntent = PendingIntent.getService(
            this,
            3,
            Intent(this, ServerService::class.java).setAction(actionRestart),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val statusText = if (isRunning) "Running" else "Stopped"
        val endpoint = "${config.host}:${config.port}"
        val builder = NotificationCompat.Builder(this, channelId)
            .setSmallIcon(R.drawable.ic_launcher)
            .setContentTitle("NoDPI Server")
            .setContentText("Status: $statusText · $endpoint")
            .setSubText(endpoint)
            .setStyle(
                NotificationCompat.BigTextStyle()
                    .setBigContentTitle("NoDPI Server")
                    .bigText("Status: $statusText\nEndpoint: $endpoint\n${trafficSummary}\n$message")
            )
            .setContentIntent(openIntent)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setShowWhen(false)
        if (showProgress) {
            builder.setProgress(0, 0, true)
        }
        if (isRunning) {
            builder.addAction(0, "Stop", stopIntent)
            builder.addAction(0, "Restart", restartIntent)
        } else {
            builder.addAction(0, "Start", startIntent)
        }
        return builder.build()
    }

    private fun ensureMonitor() {
        synchronized(monitorLock) {
            if (monitorActive) return
            monitorActive = true
            Thread {
                while (monitorActive) {
                    val config = configStore.load()
                    val nowRunning = serverManager.isRunning()
                    if (nowRunning != running) {
                        running = nowRunning
                        updateNotification(
                            if (running) "Server running" else "Server stopped",
                            running,
                            false,
                            config
                        )
                    }
                    updateTrafficSummary()
                    updateNotification(
                        if (running) "Server running" else "Server stopped",
                        running,
                        false,
                        config
                    )
                    Thread.sleep(2000)
                }
            }.start()
        }
    }

    private fun updateTrafficSummary() {
        val uid = Process.myUid()
        val rx = TrafficStats.getUidRxBytes(uid)
        val tx = TrafficStats.getUidTxBytes(uid)
        if (rx < 0 || tx < 0) {
            trafficSummary = "Traffic: n/a"
            return
        }
        val now = System.currentTimeMillis()
        val rate = if (lastTimeMs > 0) (now - lastTimeMs).coerceAtLeast(1) else 0
        val deltaRx = if (lastRx >= 0) rx - lastRx else 0
        val deltaTx = if (lastTx >= 0) tx - lastTx else 0
        val speedIn = if (rate > 0) deltaRx * 1000 / rate else 0
        val speedOut = if (rate > 0) deltaTx * 1000 / rate else 0
        trafficSummary = "Traffic: RX ${formatBytes(rx)} (" +
            "${formatBytes(speedIn)}/s), TX ${formatBytes(tx)} (" +
            "${formatBytes(speedOut)}/s)"
        lastRx = rx
        lastTx = tx
        lastTimeMs = now
    }

    private fun formatBytes(value: Long): String {
        val units = arrayOf("B", "KB", "MB", "GB")
        var size = value.toDouble()
        var unit = 0
        while (size >= 1024.0 && unit < units.lastIndex) {
            size /= 1024.0
            unit += 1
        }
        return String.format("%.1f %s", size, units[unit])
    }
}
