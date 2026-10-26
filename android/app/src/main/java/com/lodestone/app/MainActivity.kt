package com.lodestone.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.server.LodestoneService
import com.lodestone.app.state.StateStore
import org.json.JSONArray
import org.json.JSONObject
import com.sun.jna.Pointer

class MainActivity : ComponentActivity() {
    private var engine: Pointer? = null
    private var initError: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        try {
            engine = NativeBridge.engineNew(cacheDir.absolutePath)
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "Engine creation failed", e)
            initError = "${e.javaClass.simpleName}: ${e.message}"
        }
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    LodestoneApp(engine = engine, initError = initError)
                }
            }
        }
    }

    override fun onDestroy() {
        engine?.let { NativeBridge.engineFree(it) }
        engine = null
        super.onDestroy()
    }
}

@Composable
fun LodestoneApp(engine: Pointer?, initError: String?) {
    var selectedTab by remember { mutableStateOf(0) }
    val tabs = listOf("Dashboard", "Instances", "Console", "Settings", "Access")

    Column(modifier = Modifier.fillMaxSize()) {
        // Tab row
        ScrollableTabRow(selectedTabIndex = selectedTab, edgePadding = 8.dp) {
            tabs.forEachIndexed { index, title ->
                Tab(selected = selectedTab == index, onClick = { selectedTab = index }) {
                    Text(title, modifier = Modifier.padding(8.dp))
                }
            }
        }

        // Content
        when (selectedTab) {
            0 -> DashboardScreen(engine)
            1 -> InstancesScreen(engine)
            2 -> ConsoleScreen(engine)
            3 -> SettingsScreen(engine)
            4 -> AccessScreen(engine)
        }
    }
}

@Composable
fun DashboardScreen(engine: Pointer?) {
    var status by remember { mutableStateOf("idle") }
    var instances by remember { mutableStateOf<List<String>>(emptyList()) }

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("Dashboard", style = MaterialTheme.typography.headlineMedium)
        if (initError != null) {
            Text("Error: $initError", modifier = Modifier.padding(top = 8.dp))
        }
        Text("Status: $status", modifier = Modifier.padding(top = 8.dp))

        // Instance status
        val states = StateStore.snapshot()
        if (states.isNotEmpty()) {
            states.forEach { (id, state) ->
                Text("$id: $state", modifier = Modifier.padding(top = 4.dp))
            }
        } else {
            Text("No instances. Create one in the Instances tab.", modifier = Modifier.padding(top = 8.dp))
        }

        // Buttons
        Row(modifier = Modifier.padding(top = 16.dp)) {
            Button(onClick = {
                engine?.let { e ->
                    val listJson = NativeBridge.listInstances(e)
                    if (listJson != null) {
                        try {
                            val arr = JSONArray(listJson)
                            val ids = mutableListOf<String>()
                            for (i in 0 until arr.length()) {
                                ids.add(arr.getJSONObject(i).getString("id"))
                            }
                            instances = ids
                            status = "found ${ids.size} instances"
                        } catch (ex: Exception) {
                            status = "error: ${ex.message}"
                        }
                    } else {
                        status = "no instances"
                    }
                } ?: run { status = "engine null" }
            }) {
                Text("Refresh")
            }
            Spacer(modifier = Modifier.width(8.dp))
            Button(onClick = {
                val intent = android.content.Intent(
                    androidx.core.content.ContextCompat.getSystemService(
                        /* context */ null, android.content.Context::class.java
                    ) as? android.content.Context ?: return@Button,
                    com.lodestone.app.server.LodestoneService::class.java
                )
            }) {
                Text("Start Service")
            }
        }
    }
}

@Composable
fun InstancesScreen(engine: Pointer?) {
    var serverType by remember { mutableStateOf("paper") }
    var version by remember { mutableStateOf("") }
    var instances by remember { mutableStateOf<List<org.json.JSONObject>>(emptyList()) }
    var status by remember { mutableStateOf("") }

    LazyColumn(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        item {
            Text("Instance Manager", style = MaterialTheme.typography.headlineMedium)
            Spacer(modifier = Modifier.height(8.dp))

            // Server type selector
            Text("Server Type:")
            Row(modifier = Modifier.padding(top = 4.dp)) {
                listOf("paper", "fabric", "vanilla", "forge").forEach { type ->
                    Button(
                        onClick = { serverType = type },
                        modifier = Modifier.padding(end = 4.dp),
                        colors = if (serverType == type) ButtonDefaults.buttonColors() else ButtonDefaults.outlinedButtonColors()
                    ) {
                        Text(type)
                    }
                }
            }

            // Version input
            OutlinedTextField(
                value = version,
                onValueChange = { version = it },
                label = { Text("Version (e.g. 1.21.4)") },
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp)
            )

            // Create instance button
            Button(onClick = {
                engine?.let { e ->
                    if (version.isNotEmpty()) {
                        val spec = org.json.JSONObject().apply {
                            put("id", "instance-${System.currentTimeMillis()}")
                            put("server_type", serverType)
                            put("version", version)
                            put("work_dir", "/data/data/com.lodestone.app/files/instances/instance-${System.currentTimeMillis()}")
                        }
                        val rc = NativeBridge.createInstance(e, spec.toString())
                        status = if (rc == 0) "Created!" else "Failed (rc=$rc)"
                    } else {
                        status = "Enter a version"
                    }
                } ?: run { status = "engine null" }
            }, modifier = Modifier.padding(top = 8.dp)) {
                Text("Create Instance")
            }

            Text(status, modifier = Modifier.padding(top = 8.dp))

            // List instances
            Text("Instances:", modifier = Modifier.padding(top = 16.dp))
        }

        // Refresh instances
        item {
            Button(onClick = {
                engine?.let { e ->
                    val listJson = NativeBridge.listInstances(e)
                    if (listJson != null) {
                        try {
                            val arr = JSONArray(listJson)
                            val list = mutableListOf<org.json.JSONObject>()
                            for (i in 0 until arr.length()) {
                                list.add(arr.getJSONObject(i))
                            }
                            instances = list
                        } catch (ex: Exception) {
                            // ignore
                        }
                    }
                }
            }) {
                Text("Refresh Instances")
            }
        }

        items(instances) { inst ->
            val id = inst.getString("id")
            val state = inst.getString("state")
            val st = inst.getString("server_type")
            Card(modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text("$id ($st) - $state")
                    Row(modifier = Modifier.padding(top = 8.dp)) {
                        Button(onClick = {
                            engine?.let { e -> NativeBridge.start(e, id) }
                        }) { Text("Start") }
                        Spacer(modifier = Modifier.width(8.dp))
                        Button(onClick = {
                            engine?.let { e -> NativeBridge.stop(e, id) }
                        }) { Text("Stop") }
                        Spacer(modifier = Modifier.width(8.dp))
                        Button(onClick = {
                            engine?.let { e -> NativeBridge.deleteInstance(e, id) }
                        }) { Text("Delete") }
                    }
                }
            }
        }
    }
}

@Composable
fun ConsoleScreen(engine: Pointer?) {
    var consoleOutput by remember { mutableStateOf("") }
    var command by remember { mutableStateOf("") }

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("Console", style = MaterialTheme.typography.headlineMedium)
        Surface(
            modifier = Modifier.fillMaxWidth().weight(1f).padding(top = 8.dp),
            color = MaterialTheme.colorScheme.surfaceVariant
        ) {
            LazyColumn(modifier = Modifier.padding(8.dp)) {
                item {
                    Text(consoleOutput, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        Row(modifier = Modifier.padding(top = 8.dp)) {
            OutlinedTextField(
                value = command,
                onValueChange = { command = it },
                label = { Text("Command") },
                modifier = Modifier.weight(1f)
            )
            Spacer(modifier = Modifier.width(8.dp))
            Button(onClick = {
                // Send command to first instance
                engine?.let { e ->
                    val listJson = NativeBridge.listInstances(e)
                    if (listJson != null) {
                        try {
                            val arr = JSONArray(listJson)
                            if (arr.length() > 0) {
                                val id = arr.getJSONObject(0).getString("id")
                                NativeBridge.sendConsole(e, id, command)
                                consoleOutput += "> $command\n"
                                command = ""
                            }
                        } catch (ex: Exception) { }
                    }
                }
            }) {
                Text("Send")
            }
        }
    }
}

@Composable
fun SettingsScreen(engine: Pointer?) {
    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("Settings", style = MaterialTheme.typography.headlineMedium)
        Text("Server settings, tuning profiles, and device info.", modifier = Modifier.padding(top = 8.dp))
        Text("Use the Access tab for network and crossplay settings.", modifier = Modifier.padding(top = 4.dp))
    }
}

@Composable
fun AccessScreen(engine: Pointer?) {
    var crossplayEnabled by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("idle") }

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("Access", style = MaterialTheme.typography.headlineMedium)
        Text("LAN address: 192.168.x.x:25565", modifier = Modifier.padding(top = 8.dp))
        Text("Remote API: 127.0.0.1:7300", modifier = Modifier.padding(top = 4.dp))

        Text("Bedrock Crossplay (Geyser)", modifier = Modifier.padding(top = 16.dp))
        Switch(
            checked = crossplayEnabled,
            onCheckedChange = { checked ->
                crossplayEnabled = checked
                if (checked && engine != null) {
                    val listJson = NativeBridge.listInstances(engine)
                    if (listJson != null) {
                        try {
                            val arr = JSONArray(listJson)
                            if (arr.length() > 0) {
                                val instanceId = arr.getJSONObject(0).getString("id")
                                val opts = JSONObject().apply {
                                    put("geyser", true)
                                    put("floodgate", true)
                                    put("via_version", true)
                                    put("via_backwards", false)
                                }
                                val rc = NativeBridge.enableCrossplay(engine, instanceId, opts.toString())
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

        val token = StateStore.snapshot()["remote_api_token"]
        if (token != null) {
            Text("Remote API token: $token", modifier = Modifier.padding(top = 16.dp))
        }
    }
}