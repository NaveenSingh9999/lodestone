package com.lodestone.app.state

import android.content.Context
import android.content.SharedPreferences

object StateStore {
    private const val PREFS = "lodestone_state"

    private var prefs: SharedPreferences? = null

    fun init(context: Context) {
        prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    fun update(instanceId: String, status: String) {
        prefs?.edit()?.putString(instanceId, status)?.apply()
    }

    fun remove(instanceId: String) {
        prefs?.edit()?.remove(instanceId)?.apply()
    }

    fun snapshot(): Map<String, String> {
        return prefs?.all
            ?.filterValues { it is String }
            ?.mapValues { it.value as String }
            ?: emptyMap()
    }

    fun anyRunning(): Boolean {
        return prefs?.all?.values?.any { it is String && it == "Running" } == true
    }
}
