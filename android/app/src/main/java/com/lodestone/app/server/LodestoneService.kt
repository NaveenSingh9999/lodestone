package com.lodestone.app.server

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

class LodestoneService : Service() {
    companion object {
        init { System.loadLibrary("lodestone_ffi") }
        const val CHANNEL_ID = "lodestone_server"
        const val NOTIF_ID = 1
    }

    override fun onCreate() {
        super.onCreate()
        createChannel()
        startForeground(NOTIF_ID, buildNotification("Lodestone server starting"))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Native engine lifecycle is driven here via the JNI bridge; the
        // :server process hosts the JVM via dlopen + JLI_Launch (spec §4.3).
        return START_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(NotificationManager::class.java)
            mgr.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Server", NotificationManager.IMPORTANCE_LOW)
            )
        }
    }

    private fun buildNotification(text: String): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Lodestone")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .build()
}
