@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package com.lodestone.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.Instance
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun InstancesContent(model: AppModel) {
    val scope = rememberCoroutineScope()
    var sheetFor by remember { mutableStateOf<String?>(null) }
    var confirmDelete by remember { mutableStateOf<Instance?>(null) }
    var crashReport by remember { mutableStateOf<String?>(null) }

    if (model.showCreateInstance) {
        CreateInstanceDialog(model)
    }

    val list = model.instances
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (list.isEmpty()) {
            item {
                EmptyHint(
                    "No server instances yet.\nCreate one, pick a version, and Lodestone downloads + provisions everything.",
                )
            }
        } else {
            items(list.size) { idx ->
                val inst = list[idx]
                InstanceCard(model, inst) { sheetFor = inst.id }
            }
        }
    }

    sheetFor?.let { id ->
        list.firstOrNull { it.id == id }?.let { inst ->
            ModalBottomSheet(onDismissRequest = { sheetFor = null }) {
                InstanceSheet(
                    model,
                    inst,
                    onDismiss = { sheetFor = null },
                    onCrash = {
                        scope.launch {
                            model.withBusy {
                                val (_report, _err): Pair<String?, String?> = withContext(Dispatchers.IO) { model.analyzeCrashNow(inst.id) }
                                crashReport = _report
                                if (_err != null) model.toast = _err
                            }
                        }
                        sheetFor = null
                    },
                    onDelete = {
                        if (inst.state == "Running" || inst.state == "Starting") {
                            model.toast = "Stop the instance before deleting it."
                        } else {
                            confirmDelete = inst
                        }
                    },
                )
            }
        }
    }

    confirmDelete?.let { inst ->
        AlertDialog(
            onDismissRequest = { confirmDelete = null },
            title = { Text("Delete instance") },
            text = { Text("Remove ${inst.id}? The folder keeps its files, but the engine forgets it.") },
            confirmButton = {
                TextButton(onClick = {
                    val target = inst
                    confirmDelete = null
                    sheetFor = null
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.deleteInstanceNow(target.id) }
                            model.toast = err ?: "Deleted ${target.id}"
                            model.refreshNow()
                        }
                    }
                }) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = null }) { Text("Cancel") }
            },
        )
    }

    crashReport?.let { report ->
        AlertDialog(
            onDismissRequest = { crashReport = null },
            title = { Text("Crash report") },
            text = {
                Column {
                    Text(
                        crashBody(report),
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = { crashReport = null }) { Text("Close") }
            },
        )
    }
}

private fun crashBody(report: String): String = try {
    val o = org.json.JSONObject(report)
    val hint = o.optString("hint")
    val tail = o.optJSONArray("tail") ?: org.json.JSONArray()
    val lines = buildString {
        val n = minOf(tail.length(), 10)
        for (i in tail.length() - n until tail.length()) {
            append(tail.optString(i))
            append('\n')
        }
    }
    (if (lines.isBlank()) "(no output captured)\n" else lines) +
        "\nHint: $hint"
} catch (_: Exception) {
    report
}

@Composable
private fun InstanceCard(model: AppModel, inst: Instance, onMore: () -> Unit) {
    val scope = rememberCoroutineScope()
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(inst.id, style = MaterialTheme.typography.titleMedium)
                    Text(
                        "${inst.serverType} · ${inst.version}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                StateChip("", inst.state)
            }
            Row(
                modifier = Modifier.padding(top = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (inst.state == "Stopped" || inst.state == "Crashed") {
                    TextButton(onClick = {
                        scope.launch {
                            model.withBusy {
                                val err = withContext(Dispatchers.IO) { model.startNow(inst.id) }
                                model.toast = err ?: "${inst.id} starting…"
                                model.refreshNow()
                            }
                        }
                    }) { Icon(Icons.Rounded.PlayArrow, contentDescription = null); Text("Start") }
                }
                if (inst.state == "Running" || inst.state == "Starting") {
                    TextButton(onClick = {
                        scope.launch {
                            model.withBusy {
                                val err = withContext(Dispatchers.IO) { model.stopNow(inst.id) }
                                model.toast = err ?: "${inst.id} stopped"
                                model.refreshNow()
                            }
                        }
                    }) { Icon(Icons.Rounded.Stop, contentDescription = null); Text("Stop") }
                }
                TextButton(onClick = onMore) {
                    Icon(Icons.Rounded.Description, contentDescription = null)
                    Text("Manage")
                }
            }
        }
    }
}

@Composable
private fun InstanceSheet(
    model: AppModel,
    inst: Instance,
    onDismiss: () -> Unit,
    onCrash: () -> Unit,
    onDelete: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    Column(modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)) {
        Text(
            inst.id,
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
        )
        Text(
            "${inst.serverType} · ${inst.version} · ${inst.state}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        SheetAction("Start", "Boots the server process") {
            scope.launch {
                model.withBusy {
                    val err = withContext(Dispatchers.IO) { model.startNow(inst.id) }
                    model.toast = err ?: "${inst.id} starting…"
                    model.refreshNow()
                }
            }
            onDismiss()
        }
        SheetAction("Stop", "Clean shutdown; worlds are saved") {
            scope.launch {
                model.withBusy {
                    val err = withContext(Dispatchers.IO) { model.stopNow(inst.id) }
                    model.toast = err ?: "${inst.id} stopped"
                    model.refreshNow()
                }
            }
            onDismiss()
        }
        SheetAction("Crash report", "Inspect the last console output") { onCrash() }
        SheetAction("Backups", "Create, restore, or delete backups") {
            model.tab = 4
            model.settingsSub = "backups"
            onDismiss()
        }
        SheetAction(
            "Delete",
            if (inst.state == "Running" || inst.state == "Starting") "Stop the instance first"
            else "Forget this instance",
            enabled = !(inst.state == "Running" || inst.state == "Starting"),
        ) { onDelete() }
    }
}

@Composable
fun SheetAction(title: String, supporting: String, enabled: Boolean = true, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(supporting) },
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 0.dp)
            .clickable(enabled = enabled) { onClick() },
    )
}

private val SUPPORTED_TYPES = listOf("vanilla" to "Vanilla", "paper" to "Paper")
private val ALL_TYPES = listOf(
    "vanilla" to "Vanilla",
    "paper" to "Paper",
    "fabric" to "Fabric",
    "quilt" to "Quilt",
    "forge" to "Forge",
    "neoforge" to "NeoForge",
    "purpur" to "Purpur",
    "folia" to "Folia",
)

private val NAME_RE = Regex("[A-Za-z0-9_-]{2,40}")

@Composable
private fun CreateInstanceDialog(model: AppModel) {
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("inst-" + (System.currentTimeMillis() / 1000 % 100000)) }
    var type by remember { mutableStateOf("paper") }
    var version by remember { mutableStateOf("") }
    var versions by remember { mutableStateOf<List<String>?>(null) }
    var versionsErr by remember { mutableStateOf<String?>(null) }
    var loadingVersions by remember { mutableStateOf(false) }

    val nameOk = NAME_RE.matches(name.trim())
    val versionOk = version.trim().length >= 3

    AlertDialog(
        onDismissRequest = { model.showCreateInstance = false },
        title = { Text("New instance") },
        text = {
            Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
                Text("Type", style = MaterialTheme.typography.labelLarge)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.padding(top = 4.dp)) {
                    ALL_TYPES.take(4).forEach { (id, label) ->
                        FilterChip(
                            selected = type == id,
                            onClick = {
                                type = id
                                versions = null
                            },
                            enabled = id == "vanilla" || id == "paper",
                            label = { Text(label) },
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.padding(top = 4.dp)) {
                    ALL_TYPES.drop(4).forEach { (id, label) ->
                        FilterChip(
                            selected = type == id,
                            onClick = {
                                type = id
                                versions = null
                            },
                            enabled = id == "vanilla" || id == "paper",
                            label = { Text(label) },
                        )
                    }
                }
                Text(
                    "Vanilla and Paper are available today; Fabric/Forge ship as the catalog lands in-device.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name") },
                    singleLine = true,
                    isError = !nameOk,
                    modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                )
                OutlinedTextField(
                    value = version,
                    onValueChange = { version = it },
                    label = { Text("Minecraft version") },
                    singleLine = true,
                    supportingText = { Text("e.g. 1.21.4 — check the list button") },
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                Row(
                    modifier = Modifier.padding(top = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    TextButton(onClick = {
                        loadingVersions = true
                        versionsErr = null
                        scope.launch {
                            val (list, err): Pair<List<String>?, String?> = withContext(Dispatchers.IO) { model.versionsNow(type) }
                            versions = list
                            versionsErr = err
                            loadingVersions = false
                        }
                    }) {
                        Text(if (versions == null) "List versions" else "Hide list")
                    }
                    if (loadingVersions) {
                        androidx.compose.material3.CircularProgressIndicator(
                            modifier = Modifier.padding(2.dp),
                            strokeWidth = 2.dp,
                        )
                    }
                }
                versionsErr?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                if (versions != null) {
                    Column(modifier = Modifier.padding(top = 4.dp, bottom = 4.dp)) {
                        versions!!.take(40).forEach { v ->
                            Text(
                                v,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                modifier = Modifier.fillMaxWidth().clickable {
                                    version = v
                                }.padding(6.dp),
                            )
                        }
                        if (versions!!.size > 40) {
                            Text(
                                "…and ${versions!!.size - 40} more — type your version.",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = nameOk && versionOk,
                onClick = {
                    val n = name.trim()
                    val v = version.trim()
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.createInstanceNow(n, type, v) }
                            if (err == null) {
                                model.showCreateInstance = false
                                model.toast = "Creating $n…"
                                model.refreshNow()
                            } else {
                                model.toast = err
                            }
                        }
                    }
                },
            ) { Text("Create") }
        },
        dismissButton = {
            TextButton(onClick = { model.showCreateInstance = false }) { Text("Cancel") }
        },
    )
}
