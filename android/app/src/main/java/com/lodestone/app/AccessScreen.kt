package com.lodestone.app

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.server.LodestoneService
import com.lodestone.app.state.StateStore
import org.json.JSONArray
import org.json.JSONObject

@Composable
fun AccessScreen(engine: com.sun.jna.Pointer?) {
    var crossplayEnabled by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("idle") }
    var instanceId by remember { mutableStateOf("") }

    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Top
    ) {
        Text("Access", style = MaterialTheme.typography.headlineMedium)
        Text("LAN address: 192.168.x.x:25565", modifier = Modifier.padding(top = 8.dp))
        Text("Remote API: 127.0.0.1:7300", modifier = Modifier.padding(top = 4.dp))

        // Crossplay toggle
        Text("Bedrock Crossplay (Geyser)", modifier = Modifier.padding(top = 16.dp))
        Switch(
            checked = crossplayEnabled,
            onCheckedChange = { checked ->
                crossplayEnabled = checked
                if (checked && engine != null) {
                    // List instances and enable crossplay on the first one.
                    val listJson = NativeBridge.listInstances(engine)
                    if (listJson != null) {
                        try {
                            val arr = JSONArray(listJson)
                            if (arr.length() > 0) {
                                instanceId = arr.getJSONObject(0).getString("id")
                                val opts = JSONObject().apply {
                                    put("geyser", true)
                                    put("floodgate", true)
                                    put("via_version", true)
                                    put("via_backwards", false)
                                }
                                val rc = NativeBridge.enableCrossplay(
                                    engine, instanceId, opts.toString()
                                )
                                status = if (rc == 0) "crossplay enabled" else "failed (rc=$rc)"
                            } else {
                                status = "no instances"
                            }
                        } catch (e: Exception) {
                            status = "error: ${e.message}"
                        }
                    }
                }
            }
        )
        Text(status, modifier = Modifier.padding(top = 8.dp))

        // Remote API token display
        val token = StateStore.snapshot()["remote_api_token"]
        if (token != null) {
            Text("Remote API token: $token", modifier = Modifier.padding(top = 16.dp))
        }
    }
}
