package com.lodestone.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.VpnKey
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.state.AppState
import com.lodestone.app.state.EngineHolder

@Composable
fun DashboardContent(model: AppModel) {
    LaunchedEffect(Unit) {
        model.refreshDeviceAsync()
        model.refreshLanAsync()
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item { EngineCard(model) }
        item { InstancesCard(model) }
        item { DeviceCard(model) }
        item { NetworkCard(model) }
    }
}

@Composable
private fun EngineCard(model: AppModel) {
    val err = EngineHolder.initError
    val engine = EngineHolder.engine
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Engine",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                if (engine != null) {
                    StateChip("Engine", "Running")
                }
            }
            if (err != null) {
                Text(
                    "Native engine failed to start: $err",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
                Text(
                    "Reinstall the APK; native lib must match your device ABI (arm64).",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else if (engine == null) {
                Text(
                    "Native engine not ready yet…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Text(
                    "Data folder: ${model.filesRoot.absolutePath}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                val snackbarHint = "Server work dir lives under instances/ here."
                Text(
                    snackbarHint,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun InstancesCard(model: AppModel) {
    val states = AppState.snapshot()
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Servers", style = MaterialTheme.typography.titleMedium)
            Text(
                "Start and stop every instance from anywhere — even from the lock-screen notification.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.padding(6.dp))
            if (states.isEmpty()) {
                EmptyHint(
                    "No instances yet.",
                    action = {
                        TextButton(onClick = {
                            model.showCreateInstance = true
                            model.tab = 1
                        }) { Text("Create your first instance") }
                    }
                )
            } else {
                states.entries.chunked(3).forEach { rowEntries ->
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        rowEntries.forEach { (id, state) ->
                            StateChip(id, state)
                        }
                    }
                }
            }
            Spacer(Modifier.padding(4.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { model.startAll() }) {
                    Icon(Icons.Rounded.PlayArrow, contentDescription = null)
                    Text("Start all")
                }
                OutlinedButton(onClick = { model.stopAll() }) {
                    Icon(Icons.Rounded.Stop, contentDescription = null)
                    Text("Stop all")
                }
            }
        }
    }
}

@Composable
private fun NetworkCard(model: AppModel) {
    val clipboard = LocalClipboardManager.current
    val ip = model.lanIp
    val token = AppState.getRemoteToken()
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Rounded.Wifi,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.width(12.dp))
                Text("Network", style = MaterialTheme.typography.titleMedium)
            }
            InfoRow("LAN address", "${ip ?: "unknown"} : 25565")
            Text(
                "Friends on the same Wi-Fi join at that address. Sleep proxy hands out a lobby motd when nobody is around and boots the server on connect.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (token != null) {
                Spacer(Modifier.padding(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        Icons.Rounded.VpnKey,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.tertiary,
                        modifier = Modifier.width(20.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Remote API",
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        token,
                        style = MaterialTheme.typography.labelMedium,
                        fontFamily = FontFamily.Monospace,
                        maxLines = 1,
                    )
                    TextButton(onClick = {
                        clipboard.setText(AnnotatedString(token))
                        model.toast = "Token copied"
                    }) { Text("Copy") }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Rounded.Bedtime,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.secondary,
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    "Management API: 127.0.0.1:${com.lodestone.app.server.LodestoneService.REMOTE_API_PORT}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
