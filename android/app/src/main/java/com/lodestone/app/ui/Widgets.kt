package com.lodestone.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDropDown
import androidx.compose.material.icons.rounded.DeviceThermostat
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.ui.theme.stateColor

/** Small pill showing a lifecycle state (and optionally an instance name) with its color. */
@Composable
fun StateChip(text: String?, state: String) {
    val color = stateColor(state)
    val shown = if (text.isNullOrBlank()) state else "$text · $state"
    Box(
        modifier = Modifier
            .clip(CircleShape)
            .background(color.copy(alpha = 0.18f))
    ) {
        Text(
            shown,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            style = MaterialTheme.typography.labelMedium,
            color = color,
        )
    }
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(),
        modifier = modifier.padding(bottom = 4.dp),
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
fun EmptyHint(text: String, action: (@Composable () -> Unit)? = null) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 40.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        action?.let {
            Spacer(Modifier.size(12.dp))
            it
        }
    }
}

@Composable
fun InfoRow(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth().padding(vertical = 3.dp)) {
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(16.dp))
        Text(
            value,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            fontFamily = FontFamily.Monospace,
            textAlign = TextAlign.End,
            maxLines = 1,
        )
    }
}

@Composable
fun InstancePicker(
    model: AppModel,
    selected: String,
    onPick: (String) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth()) {
        Text(if (selected.isBlank()) "Select instance" else selected)
        Spacer(Modifier.width(4.dp))
        Icon(Icons.Rounded.ArrowDropDown, contentDescription = "Choose")
    }
    DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
        val list = model.instances
        if (list.isEmpty()) {
            DropdownMenuItem(
                text = { Text("No instances yet") },
                onClick = { expanded = false },
            )
        }
        list.forEach { inst ->
            DropdownMenuItem(
                text = { Text("${inst.id}  (${inst.serverType})") },
                onClick = {
                    onPick(inst.id)
                    expanded = false
                },
            )
        }
    }
}

@Composable
fun DeviceCard(model: AppModel) {
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Rounded.Memory,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.width(12.dp))
                Text(
                    "This device",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = { model.refreshDeviceAsync() }) {
                    Icon(Icons.Rounded.Refresh, contentDescription = "Refresh device info")
                }
            }
            val d = model.device
            if (d == null) {
                Text(
                    "Reading device report…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                val used = (d.totalRamMb - d.availableRamMb).coerceAtLeast(0)
                Text("RAM", style = MaterialTheme.typography.labelMedium)
                LinearProgressIndicator(
                    progress = {
                        if (d.totalRamMb == 0L) 0f
                        else (used.toFloat() / d.totalRamMb).coerceIn(0f, 1f)
                    },
                    modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                )
                InfoRow("Free RAM", "${d.availableRamMb} of ${d.totalRamMb} MB")
                InfoRow("CPU cores", "${d.cores}")
                if (d.maxFreqMhz > 0) InfoRow("Max clock", "${d.maxFreqMhz} MHz")
                Row(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 3.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        "Thermals",
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Icon(
                        Icons.Rounded.DeviceThermostat,
                        contentDescription = null,
                        tint = thermalTint(d.thermalLevel),
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    val tempSuffix = d.temperatureC?.let { t -> " (${java.lang.String.format("%.0f", t)}°C)" } ?: ""
                    Text(
                        thermalLabel(d.thermalLevel) + tempSuffix,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                if (d.batteryPercent != null) {
                    InfoRow(
                        "Battery",
                        "${d.batteryPercent}%" + (if (d.charging) " charging" else " on battery"),
                    )
                }
                if (d.lowMemory) {
                    Text(
                        "Low-memory device: prefer small heaps (Tuning → Low).",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

internal fun thermalLabel(level: Int): String = when (level) {
    0 -> "Nominal"
    1 -> "Light"
    2 -> "Moderate"
    3 -> "Severe"
    4 -> "Critical"
    else -> "Emergency"
}

internal fun thermalTint(level: Int): Color = when (level) {
    0 -> Color(0xFF7FBF6A)
    1 -> Color(0xFFE4C284)
    else -> Color(0xFFE57368)
}

fun settingsSubTitle(sub: String): String = when (sub) {
    "server" -> "Server settings"
    "network" -> "Network"
    "plugins" -> "Plugins & mods"
    "backups" -> "Backups"
    "device" -> "This device"
    "about" -> "About"
    else -> "Settings"
}
