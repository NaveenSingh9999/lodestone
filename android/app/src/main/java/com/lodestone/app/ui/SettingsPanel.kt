package com.lodestone.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Backup
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Extension
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Save
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.BackupItem
import com.lodestone.app.ContentHit
import com.lodestone.app.Instance
import com.lodestone.app.contentDirFor
import com.lodestone.app.fmtBytes
import com.lodestone.app.state.AppState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

@Composable
fun SettingsContent(model: AppModel) {
    BackHandler(enabled = model.settingsSub != null) { model.settingsSub = null }
    when (model.settingsSub) {
        "server" -> ServerSettings(model)
        "network" -> NetworkSettings(model)
        "plugins" -> PluginsSettings(model)
        "backups" -> BackupsSettings(model)
        "device" -> DeviceSettings(model)
        "about" -> AboutSettings(model)
        else -> SettingsRoot(model)
    }
}

// ---------- root ----------

@Composable
private fun SettingsRoot(model: AppModel) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp, 12.dp, 16.dp, 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Edit, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "Server settings",
                supporting = "Edit server.properties, apply performance tuning",
                onClick = { model.settingsSub = "server" },
            )
        }
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Wifi, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "Network",
                supporting = "LAN join address, sleep proxy, remote management",
                onClick = { model.settingsSub = "network" },
            )
        }
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Extension, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "Plugins & mods",
                supporting = "Search Modrinth and install checksum-verified files",
                onClick = { model.settingsSub = "plugins" },
            )
        }
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Backup, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "Backups",
                supporting = "Create, restore, and delete world backups",
                onClick = { model.settingsSub = "backups" },
            )
        }
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Memory, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "This device",
                supporting = "RAM, thermals, battery — capacity planner",
                onClick = { model.settingsSub = "device" },
            )
        }
        item {
            SettingsNav(
                icon = { Icon(Icons.Rounded.Info, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                title = "About",
                supporting = "Version, stack, and source",
                onClick = { model.settingsSub = "about" },
            )
        }
    }
}

@Composable
private fun SettingsNav(
    icon: @Composable () -> Unit,
    title: String,
    supporting: String,
    onClick: () -> Unit,
) {
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.clickable { onClick() }.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            icon()
            Spacer(Modifier.width(16.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(
                    supporting,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                Icons.Rounded.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

// ---------- server settings (properties + tuning) ----------

private data class PropRow(val comment: Boolean, val key: String, val value: String, val raw: String)

private fun parseProps(content: String): List<PropRow> = content.lines().map { line ->
    val t = line.trim()
    if (t.startsWith("#") || t.isBlank() || !t.contains('=')) {
        PropRow(true, "", "", line)
    } else {
        val i = t.indexOf('=')
        PropRow(false, t.substring(0, i), t.substring(i + 1), line)
    }
}

private fun toPropsText(rows: List<PropRow>): String = buildString {
    rows.forEach { r ->
        if (r.comment) append(r.raw) else append("${r.key}=${r.value}")
        append('\n')
    }
}

@Composable
private fun ServerSettings(model: AppModel) {
    val scope = rememberCoroutineScope()
    var instanceId by remember { mutableStateOf("") }
    var rows by remember { mutableStateOf<List<PropRow>?>(null) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var addOpen by remember { mutableStateOf(false) }
    var addKey by remember { mutableStateOf("") }
    var addValue by remember { mutableStateOf("") }
    var tuning by remember { mutableStateOf("low") }

    instanceId = if (instanceId.isBlank()) {
        model.instances.firstOrNull()?.id ?: ""
    } else instanceId

    LaunchedEffect(instanceId) {
        rows = null
        loadError = null
        if (instanceId.isNotBlank()) {
            val (content, err): Pair<String?, String?> = withContext(Dispatchers.IO) {
                model.readFileNow("instances/$instanceId/server.properties")
            }
            if (err != null) {
                loadError = err
            } else {
                rows = content?.let { parseProps(it) } ?: emptyList()
            }
        }
    }

    fun persist() {
        val list = rows ?: return
        val inst = model.instances.firstOrNull { it.id == instanceId }
        scope.launch {
            model.withBusy {
                val err = withContext(Dispatchers.IO) {
                    model.writeFileNow("instances/$instanceId/server.properties", toPropsText(list))
                }
                if (err == null) {
                    // Tuning profiles ride along if the type supports them.
                    if (inst != null && inst.serverType in setOf("paper", "folia", "purpur")) {
                        val terr = withContext(Dispatchers.IO) { model.applyTuningNow(inst, tuning) }
                        model.toast = if (terr == null) "Saved + tuned ($tuning)" else "Saved. Tuning: $terr"
                    } else {
                        model.toast = "Saved server.properties"
                    }
                    val (content, err2): Pair<String?, String?> = withContext(Dispatchers.IO) {
                        model.readFileNow("instances/$instanceId/server.properties")
                    }
                    if (err2 == null) rows = content?.let { parseProps(it) } ?: rows
                } else {
                    model.toast = err
                }
            }
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize().imePadding(),
        contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 24.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        item {
            InstancePicker(model, instanceId) { picked -> if (picked != instanceId) { instanceId = picked; rows = null } }
        }
        if (instanceId.isBlank()) {
            item { EmptyHint("Create an instance first.") }
        } else {
            item {
                loadError?.let {
                    Text(
                        "server.properties not found yet: $it\nFiles appear after the instance's first full start.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (rows == null && loadError == null) {
                    Text(
                        "Loading properties…",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            val current = rows
            if (current != null) {
            items(current.size) { i ->
                val row = current[i]
                if (row.comment) {
                    Text(
                        (row.raw.ifBlank { " " }),
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                } else {
                    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutlinedTextField(
                                value = row.key,
                                onValueChange = { nv ->
                                    rows = current.toMutableList().also { it[i] = row.copy(key = nv) }
                                },
                                label = { Text("Key") },
                                singleLine = true,
                                textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                                modifier = Modifier.weight(0.4f),
                            )
                            OutlinedTextField(
                                value = row.value,
                                onValueChange = { nv ->
                                    rows = current.toMutableList().also { it[i] = row.copy(value = nv) }
                                },
                                label = { Text("Value") },
                                singleLine = true,
                                textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                                modifier = Modifier.weight(0.6f),
                            )
                        }
                    }
                }
            }
            item {
                TextButton(onClick = { addOpen = true }) {
                    Icon(Icons.Rounded.Add, contentDescription = null)
                    Text("Add setting")
                }
            }
            item {
                val inst = model.instances.firstOrNull { it.id == instanceId }
                val tuned = inst != null &&
                    (inst.serverType == "paper" || inst.serverType == "folia" || inst.serverType == "purpur")
                if (tuned) {
                    Column {
                        Spacer(Modifier.padding(2.dp))
                        SectionLabel("Tuning profile", Modifier.padding(top = 8.dp))
                        Text(
                            "Heap sizes, GC flags, and tick-frequency budget for this device.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 6.dp)) {
                            listOf("low", "mid", "high").forEach { p ->
                                FilterChip(
                                    selected = tuning == p,
                                    onClick = { tuning = p },
                                    label = { Text(p) },
                                )
                            }
                        }
                    }
                }
            }
            item {
                TextButton(onClick = { persist() }) {
                    Icon(Icons.Rounded.Save, contentDescription = null)
                    Text("Save to the instance")
                }
            }
            }
        }
    }

    if (addOpen) {
        AlertDialog(
            onDismissRequest = { addOpen = false },
            title = { Text("Add setting") },
            text = {
                Column {
                    OutlinedTextField(
                        value = addKey,
                        onValueChange = { addKey = it },
                        label = { Text("Key") },
                        singleLine = true,
                    )
                    OutlinedTextField(
                        value = addValue,
                        onValueChange = { addValue = it },
                        label = { Text("Value") },
                        singleLine = true,
                    )
                }
            },
            confirmButton = {
                TextButton(
                    enabled = addKey.isNotBlank(),
                    onClick = {
                        val list = rows ?: emptyList()
                        rows = list + PropRow(false, addKey.trim(), addValue.trim(), "")
                        addKey = ""
                        addValue = ""
                        addOpen = false
                    },
                ) { Text("Add") }
            },
            dismissButton = { TextButton(onClick = { addOpen = false }) { Text("Cancel") } },
        )
    }
}

// ---------- network ----------

@Composable
private fun NetworkSettings(model: AppModel) {
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    val tokens = AppState.getRemoteToken()
    var allowWan by remember { mutableStateOf(AppState.getRemoteAllowWan()) }
    var instanceId by remember { mutableStateOf("") }
    var crossplayOn by remember { mutableStateOf(false) }
    var crossplayBusy by remember { mutableStateOf(false) }

    instanceId = if (instanceId.isBlank()) {
        model.instances.firstOrNull()?.id ?: ""
    } else instanceId

    Column(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp, vertical = 8.dp).verticalScroll(rememberScrollState()).imePadding(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Wifi, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(12.dp))
                    Text("Join address", style = MaterialTheme.typography.titleMedium)
                }
                InfoRow("LAN IP", model.lanIp ?: "unknown — turn Wi-Fi on")
                InfoRow("Game port", "25565")
                Text(
                    "Friends on the same Wi-Fi join ${model.lanIp ?: "(your-ip)"}:25565 on Java edition.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Bedtime, contentDescription = null, tint = MaterialTheme.colorScheme.secondary)
                    Spacer(Modifier.width(12.dp))
                    Text("Sleep proxy", style = MaterialTheme.typography.titleMedium)
                }
                Text(
                    "When no one is playing, the proxy answers pings with a 'sleeping' motd and stops the server to save battery. The first client that pings wakes it back up automatically.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "Managed automatically while any instance is running — see the notification actions to override.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Bedrock crossplay", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Adds Geyser + Floodgate so Bedrock (phone/PS5/Switch) users can join.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                InstancePicker(model, instanceId) { picked ->
                    instanceId = picked
                    crossplayOn = false
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    if (crossplayBusy) {
                        CircularProgressIndicator(modifier = Modifier.width(24.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(12.dp))
                    }
                    Text("Enable for this instance", modifier = Modifier.weight(1f))
                    Switch(
                        checked = crossplayOn,
                        onCheckedChange = { on ->
                            crossplayOn = on
                            val id = instanceId
                            if (on && id.isNotBlank()) {
                                crossplayBusy = true
                                val opts = JSONObject()
                                    .put("geyser", true)
                                    .put("floodgate", true)
                                    .put("via_version", true)
                                    .put("via_backwards", false)
                                    .toString()
                                scope.launch {
                                    model.withBusy {
                                        val err = withContext(Dispatchers.IO) { model.crossplayNow(id, opts) }
                                        model.toast = err ?: "Crossplay installed for $id"
                                    }
                                    crossplayBusy = false
                                }
                            }
                        },
                    )
                }
            }
        }
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Remote management API", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Query and control instances over HTTP/websocket on port 7300. Bearer-token auth with lockout after failed attempts.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (tokens != null) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            "Token",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(0.3f),
                        )
                        Text(
                            tokens,
                            style = MaterialTheme.typography.labelMedium,
                            fontFamily = FontFamily.Monospace,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        TextButton(onClick = {
                            clipboard.setText(AnnotatedString(tokens))
                            model.toast = "Token copied"
                        }) { Text("Copy") }
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text("Allow access beyond this phone", modifier = Modifier.weight(1f))
                    Switch(
                        checked = allowWan,
                        onCheckedChange = { on ->
                            allowWan = on
                            AppState.updateRemoteAllowWan(on)
                            scope.launch {
                                model.withBusy {
                                    val err = withContext(Dispatchers.IO) { model.restartRemoteApiNow() }
                                    model.toast = if (err == null) {
                                        if (on) "API open on 0.0.0.0:7300" else "API locked to 127.0.0.1:7300"
                                    } else {
                                        err
                                    }
                                }
                            }
                        },
                    )
                }
                TextButton(onClick = {
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.restartRemoteApiNow() }
                            model.toast = err ?: "API restarted on port 7300"
                        }
                    }
                }) { Text("Restart API") }
            }
        }
    }
}

// ---------- plugins & mods ----------

@Composable
private fun PluginsSettings(model: AppModel) {
    val scope = rememberCoroutineScope()
    var instanceId by remember { mutableStateOf("") }
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<ContentHit>>(emptyList()) }
    var searched by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(false) }
    var selectedProject by remember { mutableStateOf<ContentHit?>(null) }
    var versions by remember { mutableStateOf<List<com.lodestone.app.ContentVersion>>(emptyList()) }

    instanceId = if (instanceId.isBlank()) {
        model.instances.firstOrNull()?.id ?: ""
    } else instanceId

    val inst = model.instances.firstOrNull { it.id == instanceId }
    val subdir = inst?.let { contentDirFor(it.serverType) }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            InstancePicker(model, instanceId) { picked -> if (picked != instanceId) { instanceId = picked } }
        }
        item {
            if (inst == null) {
                EmptyHint("Pick or create an instance to install into.")
            } else if (subdir == "plugins" && inst.serverType != "paper" && inst.serverType != "purpur" && inst.serverType != "folia") {
                Text(
                    "Vanilla has no plugin folder — best install mods on Fabric/Forge or use a Paper instance.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Text(
                    "Files land in /instances/${instanceId}/$subdir and are hash-verified before install.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    label = { Text("Search Modrinth") },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                OutlinedButton(
                    enabled = query.isNotBlank() && inst != null && !loading,
                    onClick = {
                        loading = true
                        selectedProject = null
                        versions = emptyList()
                        val q = query.trim()
                        scope.launch {
                            model.withBusy {
                                val (list, err): Pair<List<ContentHit>?, String?> = withContext(Dispatchers.IO) { model.searchContentNow(q) }
                                hits = list ?: emptyList()
                                searched = true
                                if (err != null) model.toast = err
                            }
                            loading = false
                        }
                    },
                ) {
                    Icon(Icons.Rounded.Search, contentDescription = null)
                }
            }
        }
        if (loading && searched && hits.isEmpty()) {
            item { CircularProgressIndicator() }
        }
        if (searched && hits.isEmpty() && !loading) {
            item { EmptyHint("No results. Try a different search.") }
        }
        items(hits.size) { i ->
            val hit = hits[i]
            HitCard(hit, selectedProject?.projectId == hit.projectId) {
                selectedProject = hit
                versions = emptyList()
                val pid = hit.projectId
                scope.launch {
                    val (vlist, err): Pair<List<com.lodestone.app.ContentVersion>?, String?> = withContext(Dispatchers.IO) { model.contentVersionsNow(pid) }
                    versions = vlist ?: emptyList()
                    if (err != null) model.toast = err
                }
            }
        }
        if (versions.isNotEmpty()) {
            item {
                SectionLabel("Versions — tap to install")
            }
            items(versions.size) { i ->
                val v = versions[i]
                TextButton(onClick = {
                    val proj = selectedProject ?: return@TextButton
                    val target = inst
                    if (target == null) {
                        model.toast = "Pick an instance first"
                        return@TextButton
                    }
                    scope.launch {
                        model.withBusy {
                            val (path, err): Pair<String?, String?> = withContext(Dispatchers.IO) { model.installContentNow(target, proj.projectId, v.id) }
                            model.toast = err ?: "Installed ${path ?: v.number}"
                        }
                    }
                }) {
                    Text(
                        "${v.number} • ${v.loaders.ifEmpty { listOf("mc") }.joinToString()}",
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            }
        }
    }
}

@Composable
private fun HitCard(hit: ContentHit, selected: Boolean, onClick: () -> Unit) {
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier
                .clickable { onClick() }
                .padding(14.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    hit.title,
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (selected) {
                    Text(
                        "selected",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            Text(
                "${hit.downloads} downloads",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (hit.description.isNotBlank()) {
                Text(
                    hit.description,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

// ---------- backups ----------

@Composable
private fun BackupsSettings(model: AppModel) {
    val scope = rememberCoroutineScope()
    var instanceId by remember { mutableStateOf("") }
    var list by remember { mutableStateOf<List<BackupItem>>(emptyList()) }
    var loaded by remember { mutableStateOf(false) }
    var creating by remember { mutableStateOf(false) }
    var confirmRestore by remember { mutableStateOf<BackupItem?>(null) }
    var confirmDelete by remember { mutableStateOf<BackupItem?>(null) }

    instanceId = if (instanceId.isBlank()) {
        model.instances.firstOrNull()?.id ?: ""
    } else instanceId

    LaunchedEffect(instanceId) {
        if (instanceId.isNotBlank()) {
            val (backups, err): Pair<List<BackupItem>?, String?> = withContext(Dispatchers.IO) { model.backupsNow(instanceId) }
            list = backups ?: emptyList()
            loaded = true
            if (err != null) model.toast = err
        }
    }

    fun reload() {
        val id = instanceId
        scope.launch {
            val (backups, err): Pair<List<BackupItem>?, String?> = withContext(Dispatchers.IO) { model.backupsNow(id) }
            list = backups ?: emptyList()
            if (err != null) model.toast = err
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            InstancePicker(model, instanceId) { picked -> if (picked != instanceId) { instanceId = picked; loaded = false } }
        }
        item {
            TextButton(
                enabled = instanceId.isNotBlank() && !creating,
                onClick = {
                    creating = true
                    val id = instanceId
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.createBackupNow(id, "manual") }
                            model.toast = err ?: "Backup created"
                        }
                        creating = false
                        reload()
                    }
                },
            ) {
                if (creating) CircularProgressIndicator(modifier = Modifier.width(20.dp), strokeWidth = 2.dp)
                Icon(Icons.Rounded.Add, contentDescription = null)
                Text("Create backup now")
            }
        }
        if (loaded && list.isEmpty()) {
            item { EmptyHint("No backups yet for this instance.") }
        }
        items(list.size) { i ->
            val b = list[i]
            ElevatedCard(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(14.dp)) {
                    Text(
                        b.name,
                        style = MaterialTheme.typography.titleMedium,
                        fontFamily = FontFamily.Monospace,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "${fmtBytes(b.size)} • sha256 ${b.sha256.take(12)}…",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 4.dp)) {
                        TextButton(onClick = { confirmRestore = b }) { Text("Restore") }
                        TextButton(onClick = { confirmDelete = b }) { Text("Delete") }
                    }
                }
            }
        }
    }

    confirmRestore?.let { b ->
        AlertDialog(
            onDismissRequest = { confirmRestore = null },
            title = { Text("Restore backup") },
            text = { Text("Copy ${b.name} back over the instance files? Running files will be replaced on next boot.") },
            confirmButton = {
                TextButton(onClick = {
                    val id = instanceId
                    confirmRestore = null
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.restoreBackupNow(b.path, id) }
                            model.toast = err ?: "Restored"
                        }
                    }
                }) { Text("Restore") }
            },
            dismissButton = { TextButton(onClick = { confirmRestore = null }) { Text("Cancel") } },
        )
    }
    confirmDelete?.let { b ->
        AlertDialog(
            onDismissRequest = { confirmDelete = null },
            title = { Text("Delete backup") },
            text = { Text("Remove ${b.name} permanently?") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = null
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.deleteBackupNow(b.path) }
                            model.toast = err ?: "Backup deleted"
                        }
                        reload()
                    }
                }) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = null }) { Text("Cancel") } },
        )
    }
}

// ---------- device + about ----------

@Composable
private fun DeviceSettings(model: AppModel) {
    LaunchedEffect(Unit) { model.refreshDeviceAsync() }
    Column(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp, vertical = 8.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        DeviceCard(model)
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Capacity planner", style = MaterialTheme.typography.titleMedium)
                Text(
                    "A Paper server with 5 players fits on ~1 GB RAM. Prefer the low tuning profile on phones with less than 4 GB free; device thermal states above Severe throttle the JVM automatically.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun AboutSettings(model: AppModel) {
    Column(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp, vertical = 8.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Lodestone 0.3.0", style = MaterialTheme.typography.headlineSmall)
                Text(
                    "Android-first Minecraft Java server manager.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                HorizontalDivider(modifier = Modifier.padding(vertical = 10.dp))
                InfoRow("Engine", "Rust + JNA (arm64)")
                InfoRow("UI", "Jetpack Compose · Material 3")
                InfoRow("Data folder", model.filesRoot.name)
                InfoRow("Source", "github.com/NaveenSingh9999/lodestone")
            }
        }
        ElevatedCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Catalog", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Vanilla and Paper are downloadable with checksum verification today. Fabric, Forge, NeoForge, Purpur, Folia, and Quilt arrive with the same guarantee as the catalog providers come online.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
