package com.lodestone.app.server

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.state.StateStore
import org.json.JSONArray
import org.json.JSONObject

class LodestoneService : Service() {
    companion object {
        init { System.loadLibrary("lodestone_ffi") }
        const val CHANNEL_ID = "lodestone_server"
        const val NOTIF_ID = 1
        const val ACTION_START_ALL = "com.lodestone.app.ACTION_START_ALL"
        const val ACTION_STOP_ALL = "com.lodestone.app.ACTION_STOP_ALL"
    }

    private var engine: com.sun.jna.Pointer? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        StateStore.init(this)
        engine = NativeBridge.engineNew(cacheDir.absolutePath)
        engine?.let { e ->
            NativeBridge.registerEventCallback(e) { eventJson ->
                handleEvent(eventJson)
            }
        }
        startForeground(NOTIF_ID, buildNotification(StateStore.snapshot()))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START_ALL -> startAllInstances()
            ACTION_STOP_ALL -> stopAllInstances()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        engine?.let { e -> NativeBridge.engineFree(e) }
        engine = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun handleEvent(eventJson: String) {
        try {
            val obj = JSONObject(eventJson)
            val type = obj.getString("type")
            val data = obj.getJSONObject("data")
            when (type) {
                "StateChanged" -> {
                    val id = data.getString("instance_id")
                    val state = data.getString("state")
                    StateStore.update(id, state)
                    updateNotification()
                }
            }
        } catch (e: Exception) {
            // ignore malformed events
        }
    }

    private fun startAllInstances() {
        val listJson = NativeBridge.listInstances(engine) ?: return
        try {
            val arr = JSONArray(listJson)
            for (i in 0 until arr.length()) {
                val id = arr.getJSONObject(i).getString("id")
                engine?.let { NativeBridge.start(it, id) }
            }
        } catch (e: Exception) {
            // ignore parse errors
        }
    }

    private fun stopAllInstances() {
        val listJson = NativeBridge.listInstances(engine) ?: return
        try {
            val arr = JSONArray(listJson)
            for (i in 0 until arr.length()) {
                val id = arr.getJSONObject(i).getString("id")
                engine?.let { NativeBridge.stop(it, id) }
            }
        } catch (e: Exception) {
            // ignore parse errors
        }
    }

    private fun updateNotification() {
        val mgr = getSystemService(NotificationManager::class.java) as NotificationManager
        mgr.notify(NOTIF_ID, buildNotification(StateStore.snapshot()))
    }

    private fun buildNotification(states: Map<String, String>): Notification {
        val summary = when {
            states.isEmpty() -> "No instances"
            else -> states.entries.joinToString(", ") { "${it.key}=${it.value}" }
        }

        val startIntent = Intent(this, LodestoneService::class.java).apply {
            action = ACTION_START_ALL
        }
        val stopIntent = Intent(this, LodestoneService::class.java).apply {
            action = ACTION_STOP_ALL
        }
        val pendingFlags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            PendingIntent.FLAG_IMMUTABLE
        } else {
            0
        }
        val startPending = PendingIntent.getService(this, 100, startIntent, pendingFlags)
        val stopPending = PendingIntent.getService(this, 101, stopIntent, pendingFlags)

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Lodestone")
            .setContentText(summary)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .addAction(android.R.drawable.ic_media_play, "Start All", startPending)
            .addAction(android.R.drawable.ic_media_pause, "Stop All", stopPending)
            .build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(NotificationManager::class.java) as NotificationManager
            mgr.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Server", NotificationManager.IMPORTANCE_LOW)
            )
        }
    }
}
