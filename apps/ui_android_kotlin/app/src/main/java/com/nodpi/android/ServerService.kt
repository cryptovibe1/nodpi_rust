package com.nodpi.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.app.PendingIntent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import java.io.File

class ServerService : Service() {
    companion object {
        private const val channelId = "nodpi_server"
        private const val channelName = "NoDPI Server"
        private const val notificationId = 1001
        private const val actionStart = "com.nodpi.android.action.START"
        private const val actionStop = "com.nodpi.android.action.STOP"

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

        fun isRunning(): Boolean = running
    }

    private lateinit var serverManager: ServerManager
    private lateinit var configStore: ConfigStore

    override fun onCreate() {
        super.onCreate()
        val rootDir = File(filesDir, "nodpi")
        val execDir = File(codeCacheDir, "nodpi_bin")
        configStore = ConfigStore(rootDir)
        serverManager = ServerManager(this, rootDir, configStore, execDir)
        ensureChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            actionStart -> {
                startForeground(notificationId, buildNotification("Starting server", false, true))
                Thread {
                    val config = configStore.load()
                    val result = serverManager.start(config)
                    LogStore.append("[info] service start: ${result.message}")
                    running = result.success && serverManager.isRunning()
                    updateNotification(if (running) "Server running" else "Start failed", running, false)
                    if (!running) {
                        stopSelf()
                    }
                }.start()
            }
            actionStop -> {
                Thread {
                    val result = serverManager.stop()
                    LogStore.append("[info] service stop: ${result.message}")
                    running = false
                    updateNotification("Server stopped", false, false)
                    stopForeground(true)
                    stopSelf()
                }.start()
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(channelId, channelName, NotificationManager.IMPORTANCE_LOW)
        manager.createNotificationChannel(channel)
    }

    private fun updateNotification(message: String, isRunning: Boolean, showProgress: Boolean) {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(notificationId, buildNotification(message, isRunning, showProgress))
    }

    private fun buildNotification(message: String, isRunning: Boolean, showProgress: Boolean): Notification {
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
        val statusText = if (isRunning) "Running" else "Stopped"
        val builder = NotificationCompat.Builder(this, channelId)
            .setSmallIcon(R.drawable.ic_launcher)
            .setContentTitle("NoDPI Server")
            .setContentText("Status: $statusText")
            .setSubText(statusText)
            .setStyle(
                NotificationCompat.BigTextStyle()
                    .setBigContentTitle("NoDPI Server")
                    .bigText("Status: $statusText\n$message")
            )
            .setContentIntent(openIntent)
            .setOngoing(isRunning)
            .setOnlyAlertOnce(true)
        if (showProgress) {
            builder.setProgress(0, 0, true)
        }
        if (isRunning) {
            builder.addAction(0, "Stop", stopIntent)
        }
        return builder.build()
    }
}
