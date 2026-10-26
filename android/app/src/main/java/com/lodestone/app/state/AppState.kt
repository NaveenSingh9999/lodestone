package com.lodestone.app.state

import android.content.Context
import android.content.SharedPreferences

/**
 * Shared application state accessible from both the service and the activity.
 * Uses SharedPreferences for persistence and a thread-safe in-memory console
 * buffer for live output. All writes arrive on background threads (service
 * callbacks / JNA threads); all reads happen on the UI thread.
 */
object AppState {
    private const val MAX_CONSOLE_LINES = 600

    private var prefs: SharedPreferences? = null

    private val lock = Any()
    private val consoleLines = mutableListOf<String>()
    private val instanceStates = mutableMapOf<String, String>()

    // Event listeners (set by the activity). Called on the emitter thread;
    // handling must be quick and non-blocking.
    private val listeners = mutableSetOf<(String, String) -> Unit>()

    fun init(context: Context) {
        prefs = context.getSharedPreferences("lodestone_state", Context.MODE_PRIVATE)
        // Load persisted instance states.
        prefs?.all?.forEach { (k, v) ->
            if (v is String && k.startsWith("inst_")) {
                instanceStates[k.removePrefix("inst_")] = v
            }
        }
    }

    fun addListener(listener: (String, String) -> Unit) {
        synchronized(lock) { listeners.add(listener) }
    }

    fun removeListener(listener: (String, String) -> Unit) {
        synchronized(lock) { listeners.remove(listener) }
    }

    fun updateInstanceState(instanceId: String, state: String) {
        if (instanceId.isEmpty()) return
        synchronized(lock) { instanceStates[instanceId] = state }
        prefs?.edit()?.putString("inst_$instanceId", state)?.apply()
        val copy = synchronized(lock) { listeners.toList() }
        copy.forEach { it("StateChanged", instanceId) }
    }

    fun appendConsole(line: String) {
        if (line.isEmpty()) return
        synchronized(lock) {
            consoleLines.add(line)
            if (consoleLines.size > MAX_CONSOLE_LINES) {
                repeat(consoleLines.size - MAX_CONSOLE_LINES) { consoleLines.removeAt(0) }
            }
        }
        val copy = synchronized(lock) { listeners.toList() }
        copy.forEach { it("ConsoleLine", line) }
    }

    fun consoleLines(): List<String> = synchronized(lock) { consoleLines.toList() }

    fun getConsoleOutput(): String = synchronized(lock) { consoleLines.joinToString("\n") }

    fun clearConsole() {
        synchronized(lock) { consoleLines.clear() }
    }

    fun snapshot(): Map<String, String> = synchronized(lock) { instanceStates.toMap() }

    fun anyRunning(): Boolean = synchronized(lock) {
        instanceStates.values.any { it == "Running" || it == "Starting" }
    }

    fun updateRemoteToken(token: String) {
        prefs?.edit()?.putString("remote_api_token", token)?.apply()
    }

    fun getRemoteToken(): String? = prefs?.getString("remote_api_token", null)

    fun updateRemoteAllowWan(allow: Boolean) {
        prefs?.edit()?.putBoolean("remote_api_allow_wan", allow)?.apply()
    }

    fun getRemoteAllowWan(): Boolean = prefs?.getBoolean("remote_api_allow_wan", false) ?: false
}
