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
import com.lodestone.app.R
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.state.AppState
import com.lodestone.app.state.EngineHolder
import org.json.JSONObject
import java.util.concurrent.Executors

class LodestoneService : Service() {
    companion object {
        init {
            try {
                System.loadLibrary("lodestone_ffi")
            } catch (e: Throwable) {
                android.util.Log.e("Lodestone", "Failed to load native library", e)
            }
        }
        const val CHANNEL_ID = "lodestone_server"
        const val NOTIF_ID = 1
        const val ACTION_START_ALL = "com.lodestone.app.ACTION_START_ALL"
        const val ACTION_STOP_ALL = "com.lodestone.app.ACTION_STOP_ALL"
        const val ACTION_START_SLEEP_PROXY = "com.lodestone.app.ACTION_START_SLEEP_PROXY"
        const val ACTION_STOP_SLEEP_PROXY = "com.lodestone.app.ACTION_STOP_SLEEP_PROXY"
        const val SLEEP_PROXY_BIND_PORT = 25565
        const val SLEEP_PROXY_SERVER_PORT = 25565
        const val REMOTE_API_PORT = 7300
    }

    private val wakeExecutor = Executors.newSingleThreadExecutor()
    private var sleepProxyActive = false

    override fun onCreate() {
        super.onCreate()
        try {
            createChannel()
            AppState.init(this)
            // Foreground first: the engine has no network work at creation, so
            // this must never hit the 5s ANR window.
            startForeground(NOTIF_ID, buildNotification(AppState.snapshot(), false))

            if (EngineHolder.engine == null) {
                try {
                    EngineHolder.engine = NativeBridge.engineNew(filesDir.absolutePath)
                } catch (e: Throwable) {
                    android.util.Log.e("Lodestone", "Engine creation failed", e)
                    EngineHolder.initError = "${e.javaClass.simpleName}: ${e.message}"
                }
            }
            try {
                // Shared-instance semantics: the service takes over event
                // handling for the shared engine instance.
                EngineHolder.engine?.let { e ->
                    NativeBridge.registerEventCallback(e) { eventJson ->
                        handleEvent(eventJson)
                    }
                }
            } catch (e: Throwable) {
                android.util.Log.e("Lodestone", "Callback registration failed", e)
            }
            // Remote management API on the local interface only; may be
            // re-started from Settings > Network with WAN access.
            try {
                EngineHolder.engine?.let { startRemoteApiLegacy(it) }
            } catch (e: Throwable) {
                android.util.Log.e("Lodestone", "Remote API start failed", e)
            }
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "Service onCreate failed", e)
            try {
                startForeground(NOTIF_ID, buildNotification(emptyMap(), false))
            } catch (ex: Throwable) {
                android.util.Log.e("Lodestone", "Failed to display notification", ex)
            }
        }
    }

    private fun startRemoteApiLegacy(engine: com.sun.jna.Pointer) {
        val token = AppState.getRemoteToken() ?: java.util.UUID.randomUUID().toString()
        AppState.updateRemoteToken(token)
        NativeBridge.startRemoteApi(engine, token, REMOTE_API_PORT, 0)
    }

    /** Starts the management API with the stored token and WAN preference. */
    fun restartRemoteApi() {
        try {
            val engine = EngineHolder.engine ?: return
            try {
                NativeBridge.stopRemoteApi(engine)
            } catch (_: Exception) {
            }
            val token = AppState.getRemoteToken() ?: java.util.UUID.randomUUID().toString()
            AppState.updateRemoteToken(token)
            NativeBridge.startRemoteApi(
                engine,
                token,
                REMOTE_API_PORT,
                if (AppState.getRemoteAllowWan()) 1 else 0
            )
            AppState.appendConsole("» Remote API restarted on port $REMOTE_API_PORT")
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "restartRemoteApi failed", e)
            AppState.appendConsole("» Remote API restart failed: ${e.message}")
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        try {
            when (intent?.action) {
                ACTION_START_ALL -> startAllInstances()
                ACTION_STOP_ALL -> stopAllInstances()
                ACTION_START_SLEEP_PROXY -> startSleepProxy()
                ACTION_STOP_SLEEP_PROXY -> stopSleepProxy()
            }
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "onStartCommand failed", e)
        }
        return START_STICKY
    }

    override fun onDestroy() {
        try {
            stopSleepProxy()
            wakeExecutor.shutdownNow()
            // engine stays shared via EngineHolder; never freed here.
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "onDestroy failed", e)
        }
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun handleEvent(eventJson: String) {
        try {
            val obj = JSONObject(eventJson)
            val type = obj.optString("event")
            when (type) {
                "StateChanged" -> {
                    val id = obj.optString("id")
                    val state = obj.optString("state")
                    if (id.isNotEmpty() && state.isNotEmpty()) {
                        AppState.updateInstanceState(id, state)
                        val anyRunning = AppState.anyRunning()
                        try {
                            if (state == "Running") {
                                if (!sleepProxyActive) startSleepProxy()
                            } else if (!anyRunning && sleepProxyActive) {
                                stopSleepProxy()
                            }
                        } catch (e: Throwable) {
                            android.util.Log.e("Lodestone", "sleep proxy transition failed", e)
                        }
                        try {
                            updateNotification()
                        } catch (_: Throwable) {
                        }
                    }
                }
                "ConsoleLine" -> AppState.appendConsole(obj.optString("line"))
                "SleepWake" -> // A player joined the sleeping proxy: wake up
                    wakeExecutor.execute { startAllInstances() }
            }
        } catch (_: Exception) {
            // ignore malformed events
        }
    }

    private fun engine(): com.sun.jna.Pointer? = EngineHolder.engine

    private fun startAllInstances() {
        val e = engine() ?: return
        try {
            val listJson = NativeBridge.listInstances(e) ?: return
            val arr = org.json.JSONArray(listJson)
            for (i in 0 until arr.length()) {
                val obj = arr.getJSONObject(i)
                val id = obj.getString("id")
                val state = obj.optString("state")
                if (state == "Stopped" || state == "Crashed") {
                    NativeBridge.start(e, id)
                    AppState.updateInstanceState(id, "Starting")
                }
            }
        } catch (ex: Exception) {
            android.util.Log.e("Lodestone", "startAllInstances failed", ex)
        }
    }

    private fun stopAllInstances() {
        val e = engine() ?: return
        try {
            val listJson = NativeBridge.listInstances(e) ?: return
            val arr = org.json.JSONArray(listJson)
            for (i in 0 until arr.length()) {
                val id = arr.getJSONObject(i).getString("id")
                NativeBridge.stop(e, id)
                AppState.updateInstanceState(id, "Stopped")
            }
        } catch (ex: Exception) {
            android.util.Log.e("Lodestone", "stopAllInstances failed", ex)
        }
    }

    private fun startSleepProxy() {
        val e = engine() ?: return
        if (sleepProxyActive) return
        try {
            // Bind on 25565 and forward to the real server bound on the same
            // device: Java servers on Android bind 0.0.0.0:25565; the proxy
            // answers first and passes traffic through when awake.
            val rc = NativeBridge.startSleepProxy(
                e,
                "{}",
                SLEEP_PROXY_BIND_PORT,
                "127.0.0.1",
                SLEEP_PROXY_SERVER_PORT
            )
            if (rc == 0) {
                sleepProxyActive = true
                updateNotification()
            } else {
                android.util.Log.w("Lodestone", "sleep proxy start rc=$rc: ${NativeBridge.lastError()}")
            }
        } catch (ex: Throwable) {
            android.util.Log.e("Lodestone", "startSleepProxy failed", ex)
        }
    }

    private fun stopSleepProxy() {
        engine()?.let { e ->
            try {
                NativeBridge.stopSleepProxy(e)
            } catch (ex: Throwable) {
                android.util.Log.e("Lodestone", "stopSleepProxy failed", ex)
            }
        }
        if (sleepProxyActive) {
            sleepProxyActive = false
            updateNotification()
        }
    }

    private fun updateNotification() {
        try {
            val mgr = getSystemService(NotificationManager::class.java) as NotificationManager
            mgr.notify(NOTIF_ID, buildNotification(AppState.snapshot(), sleepProxyActive))
        } catch (_: Throwable) {
        }
    }

    private fun buildNotification(states: Map<String, String>, proxyActive: Boolean): Notification {
        val running = states.values.count { it == "Running" }
        val summary = when {
            states.isEmpty() -> "No instances — create one in the app"
            running == 0 -> "Idle • ${states.size} instance(s)"
            else -> "$running running • ${states.size} total"
        }

        val startIntent = Intent(this, LodestoneService::class.java).apply {
            action = ACTION_START_ALL
        }
        val stopIntent = Intent(this, LodestoneService::class.java).apply {
            action = ACTION_STOP_ALL
        }
        val proxyIntent = Intent(this, LodestoneService::class.java).apply {
            action = if (proxyActive) ACTION_STOP_SLEEP_PROXY else ACTION_START_SLEEP_PROXY
        }
        val pendingFlags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            PendingIntent.FLAG_IMMUTABLE
        } else {
            0
        }
        val startPending = PendingIntent.getService(this, 100, startIntent, pendingFlags)
        val stopPending = PendingIntent.getService(this, 101, stopIntent, pendingFlags)
        val proxyPending = PendingIntent.getService(this, 102, proxyIntent, pendingFlags)

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Lodestone")
            .setContentText(summary + (if (proxyActive) " • sleep proxy on" else ""))
            .setSmallIcon(R.drawable.ic_notification)
            .setOngoing(true)
            .addAction(android.R.drawable.ic_media_play, "Start All", startPending)
            .addAction(android.R.drawable.ic_media_pause, "Stop All", stopPending)
            .addAction(
                android.R.drawable.ic_menu_manage,
                if (proxyActive) "Sleep proxy off" else "Sleep proxy on",
                proxyPending
            )
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
