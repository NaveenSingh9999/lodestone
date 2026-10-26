package com.lodestone.app

import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.server.LodestoneService
import com.lodestone.app.state.AppState
import com.lodestone.app.state.EngineHolder
import com.lodestone.app.state.StateStore
import com.lodestone.app.ui.ConsoleContent
import com.lodestone.app.ui.DashboardContent
import com.lodestone.app.ui.FilesContent
import com.lodestone.app.ui.InstancesContent
import com.lodestone.app.ui.SettingsContent
import com.lodestone.app.ui.settingsSubTitle
import com.lodestone.app.ui.theme.LodestoneTheme
import com.sun.jna.Pointer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

// ---------- domain data ----------

data class Instance(
    val id: String,
    val serverType: String,
    val version: String,
    val state: String,
    val workDir: String,
)

data class DeviceReport(
    val totalRamMb: Long,
    val availableRamMb: Long,
    val lowMemory: Boolean,
    val cores: Int,
    val maxFreqMhz: Long,
    val thermalLevel: Int,
    val temperatureC: Double?,
    val batteryPercent: Int?,
    val charging: Boolean,
)

data class FileEntry(val name: String, val isDir: Boolean, val size: Long)

data class BackupItem(val name: String, val size: Long, val sha256: String, val path: String)

data class ContentHit(
    val projectId: String,
    val title: String,
    val description: String,
    val downloads: Long,
)

data class ContentVersion(
    val id: String,
    val name: String,
    val number: String,
    val gameVersions: List<String>,
    val loaders: List<String>,
)

fun fmtBytes(bytes: Long): String = when {
    bytes >= 1024L * 1024 * 1024 -> String.format("%.1f GB", bytes / (1024.0 * 1024 * 1024))
    bytes >= 1024L * 1024 -> String.format("%.1f MB", bytes / (1024.0 * 1024))
    bytes >= 1024L -> String.format("%.1f KB", bytes / 1024.0)
    else -> "$bytes B"
}

fun parseInstances(json: String): List<Instance> = try {
    val arr = JSONArray(json)
    (0 until arr.length()).mapNotNull { i ->
        val o = arr.getJSONObject(i)
        Instance(
            id = o.optString("id"),
            serverType = o.optString("server_type"),
            version = o.optString("version"),
            state = o.optString("state", "Stopped"),
            workDir = o.optString("work_dir"),
        )
    }
} catch (_: Exception) {
    emptyList()
}

fun parseDeviceReport(json: String): DeviceReport? = try {
    val o = JSONObject(json)
    DeviceReport(
        totalRamMb = o.optLong("total_ram_mb"),
        availableRamMb = o.optLong("available_ram_mb"),
        lowMemory = o.optBoolean("low_memory"),
        cores = o.optInt("cores"),
        maxFreqMhz = o.optLong("max_freq_mhz"),
        thermalLevel = o.optInt("thermal_level"),
        temperatureC = if (o.isNull("temperature_c")) null else o.optDouble("temperature_c"),
        batteryPercent = if (o.isNull("battery_percent")) null else o.optInt("battery_percent"),
        charging = o.optBoolean("charging"),
    )
} catch (_: Exception) {
    null
}

fun parseFileEntries(json: String): List<FileEntry> = try {
    val arr = JSONArray(json)
    (0 until arr.length()).map { i ->
        val o = arr.getJSONObject(i)
        FileEntry(
            name = o.optString("name"),
            isDir = o.optBoolean("dir"),
            size = o.optLong("size"),
        )
    }
} catch (_: Exception) {
    emptyList()
}

fun parseBackups(json: String): List<BackupItem> = try {
    val arr = JSONArray(json)
    (0 until arr.length()).map { i ->
        val o = arr.getJSONObject(i)
        BackupItem(
            name = o.optString("name"),
            size = o.optLong("size"),
            sha256 = o.optString("sha256"),
            path = o.optString("path"),
        )
    }
} catch (_: Exception) {
    emptyList()
}

fun parseContentHits(json: String): List<ContentHit> = try {
    val arr = JSONArray(json)
    (0 until arr.length()).map { i ->
        val o = arr.getJSONObject(i)
        ContentHit(
            projectId = o.optString("project_id"),
            title = o.optString("title"),
            description = o.optString("description"),
            downloads = o.optLong("downloads"),
        )
    }
} catch (_: Exception) {
    emptyList()
}

fun parseContentVersions(json: String): List<ContentVersion> = try {
    val arr = JSONArray(json)
    (0 until arr.length()).map { i ->
        val o = arr.getJSONObject(i)
        ContentVersion(
            id = o.optString("id"),
            name = o.optString("name"),
            number = o.optString("version_number"),
            gameVersions = toList(o.optJSONArray("game_versions")),
            loaders = toList(o.optJSONArray("loaders")),
        )
    }
} catch (_: Exception) {
    emptyList()
}

private fun toList(arr: JSONArray?): List<String> {
    if (arr == null) return emptyList()
    return (0 until arr.length()).map { arr.optString(it) }
}

// ---------- shared app model ----------

/**
 * Single owner of every JNI call. Blocking `*Now` functions MUST be invoked
 * from [Dispatchers.IO] (they use a thread-local error buffer in the native
 * library); UI-facing helpers wrap them and surface toasts.
 */
class AppModel(val appContext: Context) {
    val filesRoot: File = appContext.filesDir

    var tab by mutableStateOf(0)
    var settingsSub by mutableStateOf<String?>(null)
    var busy by mutableStateOf(false)
    var toast by mutableStateOf<String?>(null)

    var instances by mutableStateOf<List<Instance>>(emptyList())
    var device by mutableStateOf<DeviceReport?>(null)
    var lanIp by mutableStateOf<String?>(null)
    var showCreateInstance by mutableStateOf(false)

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    suspend fun withBusy(block: suspend () -> Unit) {
        busy = true
        try {
            block()
        } finally {
            busy = false
        }
    }

    fun dispose() {
        scope.cancel()
    }

    private fun engine(): Pointer? = EngineHolder.engine

    private inline fun <T> callEngine(mk: (Pointer) -> Pair<Int, T?>): Pair<String?, T?> {
        val e = engine() ?: return "engine not started" to null
        return try {
            val (rc, v) = mk(e)
            (if (rc == 0) null else (NativeBridge.lastError() ?: "error code $rc")) to v
        } catch (t: Throwable) {
            (t.message ?: "call failed") to null
        }
    }

    // ----- instances -----

    fun listInstancesNow(): List<Instance>? {
        val e = engine() ?: return null
        return try {
            NativeBridge.listInstances(e)?.let { parseInstances(it) }
        } catch (t: Throwable) {
            android.util.Log.e("Lodestone", "listInstances", t)
            null
        }
    }

    suspend fun refreshNow() {
        val list = withContext(Dispatchers.IO) { listInstancesNow() }
        if (list != null) {
            instances = list
            list.forEach { StateStore.update(it.id, it.state) }
        }
    }

    fun refreshAsync() {
        scope.launch { refreshNow() }
    }

    fun createInstanceNow(id: String, serverType: String, version: String): String? {
        val spec = JSONObject()
            .put("id", id)
            .put("server_type", serverType)
            .put("version", version)
            .put("work_dir", File(filesRoot, "instances/$id").absolutePath)
        val (err, _) = callEngine { e -> NativeBridge.createInstance(e, spec.toString()) to null }
        return err
    }

    fun startNow(id: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.start(e, id) to null }
        return err
    }

    fun stopNow(id: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.stop(e, id) to null }
        return err
    }

    fun deleteInstanceNow(id: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.deleteInstance(e, id) to null }
        StateStore.remove(id)
        return err
    }

    fun sendConsoleNow(id: String, command: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.sendConsole(e, id, command) to null }
        return err
    }

    fun analyzeCrashNow(id: String): Pair<String?, String?> {
        val (err, report): Pair<String?, String?> = callEngine { e ->
            NativeBridge.analyzeCrash(e, id)?.let { 0 to it } ?: (1 to null)
        }
        return report to err
    }

    fun versionsNow(serverType: String): Pair<List<String>?, String?> {
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.listVersions(e, serverType)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val list = runCatching {
            val arr = JSONArray(json!!)
            (0 until arr.length()).map { arr.getJSONObject(it).optString("id") }
        }.getOrNull()
        if (list == null) return null to "bad version list"
        return list.filter { it.isNotBlank() } to null
    }

    fun crossplayNow(instanceId: String, optionsJson: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.enableCrossplay(e, instanceId, optionsJson) to null }
        return err
    }

    // ----- remote API control -----

    fun restartRemoteApiNow(): String? {
        val e = engine() ?: return "engine not started"
        try {
            NativeBridge.stopRemoteApi(e)
        } catch (_: Throwable) {
            // already stopped is fine
        }
        val token = AppState.getRemoteToken()
            ?: java.util.UUID.randomUUID().toString()
        AppState.updateRemoteToken(token)
        val (err, _) = callEngine { en ->
            NativeBridge.startRemoteApi(
                en,
                token,
                LodestoneService.REMOTE_API_PORT,
                if (AppState.getRemoteAllowWan()) 1 else 0
            ) to null
        }
        return err
    }

    // ----- service actions -----

    fun startAll() = sendService(LodestoneService.ACTION_START_ALL)
    fun stopAll() = sendService(LodestoneService.ACTION_STOP_ALL)

    fun startService() = sendService(null)

    private fun sendService(action: String?) {
        try {
            val intent = Intent(appContext, LodestoneService::class.java)
            if (action != null) intent.action = action
            appContext.startForegroundService(intent)
        } catch (t: Throwable) {
            toast = t.message ?: "service start failed"
        }
    }

    // ----- device -----

    fun deviceNow(): DeviceReport? {
        val e = engine() ?: return null
        return try {
            NativeBridge.deviceReport(e)?.let { parseDeviceReport(it) }
        } catch (_: Throwable) {
            null
        }
    }

    fun refreshDeviceAsync() {
        scope.launch {
            device = withContext(Dispatchers.IO) { deviceNow() }
        }
    }

    fun refreshLanAsync() {
        scope.launch {
            lanIp = withContext(Dispatchers.IO) { lanAddress() }
        }
    }

    private fun lanAddress(): String? = try {
        java.net.NetworkInterface.getNetworkInterfaces().asSequence()
            .filter { nif -> nif.isUp && !nif.isLoopback }
            .flatMap { nif -> nif.inetAddresses.asSequence() }
            .filterIsInstance<java.net.Inet4Address>()
            .firstOrNull()
            ?.hostAddress
    } catch (_: Throwable) {
        null
    }

    // ----- files -----

    fun listFilesNow(rel: String): Pair<List<FileEntry>?, String?> {
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.listFiles(e, rel)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val list = json?.let { parseFileEntries(it) }
        if (list == null) return null to "bad response"
        return list to null
    }

    fun readFileNow(rel: String): Pair<String?, String?> {
        val (err, text): Pair<String?, String?> = callEngine { e ->
            NativeBridge.readFile(e, rel)?.let { 0 to it } ?: (1 to null)
        }
        return text to err
    }

    fun writeFileNow(rel: String, content: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.writeFile(e, rel, content) to null }
        return err
    }

    // ----- backups -----

    fun backupsNow(instanceId: String): Pair<List<BackupItem>?, String?> {
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.listBackups(e, instanceId)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val list = json?.let { parseBackups(it) }
        if (list == null) return null to "bad response"
        return list to null
    }

    fun createBackupNow(instanceId: String, label: String): String? {
        val source = File(filesRoot, "instances/$instanceId").absolutePath
        val (err, _) = callEngine { e ->
            NativeBridge.createBackup(e, instanceId, source, label)?.let { 0 to it } ?: (1 to null)
        }
        return err
    }

    fun restoreBackupNow(archivePath: String, instanceId: String): String? {
        val (err, _) = callEngine { e ->
            NativeBridge.restoreBackup(e, archivePath, File(filesRoot, "instances/$instanceId").absolutePath) to null
        }
        return err
    }

    fun deleteBackupNow(archivePath: String): String? {
        val (err, _) = callEngine { e -> NativeBridge.deleteBackup(e, archivePath) to null }
        return err
    }

    // ----- content (plugins / mods) -----

    fun searchContentNow(query: String): Pair<List<ContentHit>?, String?> {
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.searchContent(e, query, 12)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val list = json?.let { parseContentHits(it) }
        if (list == null) return null to "bad response"
        return list to null
    }

    fun contentVersionsNow(projectId: String): Pair<List<ContentVersion>?, String?> {
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.listContentVersions(e, projectId)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val list = json?.let { parseContentVersions(it) }
        if (list == null) return null to "bad response"
        return list to null
    }

    /**
     * Installs the primary file of `versionId` (checksum-verified) into
     * the instance's plugin/mod folder. Returns (path, error).
     */
    fun installContentNow(
        instance: Instance,
        projectId: String,
        versionId: String,
    ): Pair<String?, String?> {
        val subdir = contentDirFor(instance.serverType)
        val rel = "instances/${instance.id}/$subdir"
        val (err, json): Pair<String?, String?> = callEngine { e ->
            NativeBridge.installContent(e, projectId, versionId, rel)?.let { 0 to it } ?: (1 to null)
        }
        if (err != null) return null to err
        val path = runCatching { JSONObject(json!!).optString("path") }.getOrNull()
        return path to null
    }

    // ----- tuning -----

    fun applyTuningNow(instance: Instance, profile: String): String? {
        val (err, _) = callEngine { e ->
            NativeBridge.applyTuning(e, instance.serverType, profile, instance.workDir) to null
        }
        return err
    }
}

fun contentDirFor(serverType: String): String = when (serverType) {
    "fabric", "forge", "neoforge", "quilt" -> "mods"
    else -> "plugins"
}

// ---------- activity ----------

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        AppState.init(this)
        if (EngineHolder.engine == null) {
            try {
                EngineHolder.engine = NativeBridge.engineNew(filesDir.absolutePath)
            } catch (e: Throwable) {
                android.util.Log.e("Lodestone", "Engine creation failed", e)
                EngineHolder.initError = "${e.javaClass.simpleName}: ${e.message}"
            }
        }
        setContent {
            LodestoneTheme {
                LodestoneApp()
            }
        }
    }
}

private data class NavDest(val title: String, val icon: androidx.compose.ui.graphics.vector.ImageVector)

private val Destinations = listOf(
    NavDest("Dashboard", Icons.Rounded.Home),
    NavDest("Instances", Icons.Rounded.Apps),
    NavDest("Console", Icons.Rounded.Terminal),
    NavDest("Files", Icons.Rounded.Folder),
    NavDest("Settings", Icons.Rounded.Settings),
)

@Composable
fun LodestoneApp() {
    val context = LocalContext.current
    val model = remember { AppModel(context.applicationContext) }

    DisposableEffect(model) {
        onDispose { model.dispose() }
    }

    val snackbar = remember { SnackbarHostState() }

    LaunchedEffect(model) {
        model.withBusy { model.refreshNow() }
        model.refreshDeviceAsync()
        model.refreshLanAsync()
    }
    LaunchedEffect(model) {
        while (true) {
            delay(4000)
            if (!model.busy) model.refreshNow()
        }
    }
    LaunchedEffect(model.toast) {
        val t = model.toast ?: return@LaunchedEffect
        snackbar.showSnackbar(t)
        model.toast = null
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = {
                    val title = if (model.tab == 4 && model.settingsSub != null) {
                        settingsSubTitle(model.settingsSub!!)
                    } else {
                        Destinations[model.tab].title
                    }
                    Text(title, style = MaterialTheme.typography.titleLarge)
                },
                navigationIcon = {
                    if (model.tab == 4 && model.settingsSub != null) {
                        IconButton(onClick = { model.settingsSub = null }) {
                            Icon(Icons.Rounded.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                ),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            NavigationBar {
                Destinations.forEachIndexed { index, dest ->
                    NavigationBarItem(
                        selected = model.tab == index,
                        onClick = { model.tab = index },
                        icon = { Icon(dest.icon, contentDescription = dest.title) },
                        label = { Text(dest.title) },
                    )
                }
            }
        },
        floatingActionButton = {
            if (model.tab == 1) {
                FloatingActionButton(onClick = { model.showCreateInstance = true }) {
                    Icon(Icons.Rounded.Add, contentDescription = "Create instance")
                }
            }
        },
    ) { inner ->
        Column(
            modifier = Modifier.fillMaxSize().padding(inner)
        ) {
            if (model.busy) {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
            when (model.tab) {
                0 -> DashboardContent(model)
                1 -> InstancesContent(model)
                2 -> ConsoleContent(model)
                3 -> FilesContent(model)
                else -> SettingsContent(model)
            }
        }
    }
}
