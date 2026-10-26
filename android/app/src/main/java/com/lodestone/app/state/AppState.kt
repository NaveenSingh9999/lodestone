package com.lodestone.app.state

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject

/**
 * Shared application state accessible from both the service and the activity.
 * Uses SharedPreferences for persistence and in-memory buffers for live data.
 */
object AppState {
    private var prefs: SharedPreferences? = null

    // Live console output buffer (in-memory, bounded).
    private val consoleBuffer = StringBuilder()
    private val maxConsoleLines = 500
    private var consoleLineCount = 0

    // Live instance state map.
    private val instanceStates = mutableMapOf<String, String>()

    // Event listeners (set by the activity).
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
        listeners.add(listener)
    }

    fun removeListener(listener: (String, String) -> Unit) {
        listeners.remove(listener)
    }

    fun updateInstanceState(instanceId: String, state: String) {
        instanceStates[instanceId] = state
        prefs?.edit()?.putString("inst_$instanceId", state)?.apply()
        listeners.forEach { it("StateChanged", "$instanceId=$state") }
    }

    fun appendConsole(line: String) {
        consoleBuffer.appendLine(line)
        consoleLineCount++
        if (consoleLineCount > maxConsoleLines) {
            // Trim oldest lines.
            val lines = consoleBuffer.toString().lines()
            consoleBuffer.clear()
            lines.drop(lines.size / 4).forEach { consoleBuffer.appendLine(it) }
            consoleLineCount = consoleBuffer.lines().count()
        }
        listeners.forEach { it("ConsoleLine", line) }
    }

    fun getConsoleOutput(): String = consoleBuffer.toString()

    fun clearConsole() {
        consoleBuffer.clear()
        consoleLineCount = 0
    }

    fun snapshot(): Map<String, String> = instanceStates.toMap()

    fun anyRunning(): Boolean = instanceStates.values.any { it == "Running" }

    fun updateRemoteToken(token: String) {
        prefs?.edit()?.putString("remote_api_token", token)?.apply()
    }

    fun getRemoteToken(): String? = prefs?.getString("remote_api_token", null)
}
