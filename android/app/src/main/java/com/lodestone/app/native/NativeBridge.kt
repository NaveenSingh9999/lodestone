package com.lodestone.app.native

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.ptr.LongByReference
import com.sun.jna.ptr.PointerByReference

interface LodestoneLib : Library {
    fun lodestone_engine_new(dataDir: String): Pointer
    fun lodestone_engine_free(engine: Pointer)
    fun lodestone_string_free(s: Pointer)
    fun lodestone_last_error(): String?
    fun lodestone_list_server_types(engine: Pointer, out: PointerByReference): Int
    fun lodestone_device_report(engine: Pointer, out: PointerByReference): Int
    fun lodestone_list_versions(engine: Pointer, serverType: String, out: PointerByReference): Int
    fun lodestone_create_instance(engine: Pointer, specJson: String, outId: PointerByReference): Int
    fun lodestone_list_instances(engine: Pointer, out: PointerByReference): Int
    fun lodestone_delete_instance(engine: Pointer, id: String): Int
    fun lodestone_start(engine: Pointer, id: String): Int
    fun lodestone_stop(engine: Pointer, id: String): Int
    fun lodestone_kill(engine: Pointer, id: String): Int
    fun lodestone_send_console(engine: Pointer, id: String, line: String): Int
    fun lodestone_analyze_crash(engine: Pointer, id: String, out: PointerByReference): Int
    fun lodestone_provision_runtime(engine: Pointer, javaMajor: Int, out: PointerByReference): Int
    fun lodestone_register_event_callback(engine: Pointer, cb: EventCallback?): Int
    fun lodestone_start_sleep_proxy(
        engine: Pointer,
        configJson: String,
        bindPort: Int,
        serverHost: String,
        serverPort: Int
    ): Int
    fun lodestone_stop_sleep_proxy(engine: Pointer): Int
    fun lodestone_start_remote_api(engine: Pointer, token: String, bindPort: Int, allowWan: Int): Int
    fun lodestone_stop_remote_api(engine: Pointer): Int
    fun lodestone_enable_crossplay(engine: Pointer, instanceId: String, optionsJson: String): Int
    fun lodestone_list_files(engine: Pointer, rel: String, out: PointerByReference): Int
    fun lodestone_read_file(engine: Pointer, rel: String, out: PointerByReference): Int
    fun lodestone_write_file(engine: Pointer, rel: String, content: String): Int
    fun lodestone_install_content(
        engine: Pointer,
        projectId: String,
        versionId: String,
        destDir: String,
        out: PointerByReference
    ): Int
    fun lodestone_create_backup(
        engine: Pointer,
        instanceId: String,
        sourceDir: String,
        label: String,
        outName: PointerByReference
    ): Int
    fun lodestone_list_backups(engine: Pointer, instanceId: String, out: PointerByReference): Int
    fun lodestone_restore_backup(engine: Pointer, archivePath: String, destDir: String, outCount: LongByReference?): Int
    fun lodestone_delete_backup(engine: Pointer, archivePath: String): Int
    fun lodestone_search_content(engine: Pointer, query: String, limit: Int, out: PointerByReference): Int
    fun lodestone_list_content_versions(engine: Pointer, projectId: String, out: PointerByReference): Int
    fun lodestone_apply_tuning(engine: Pointer, serverType: String, profile: String, workDir: String): Int
}

fun interface EventCallback : com.sun.jna.Callback {
    fun onEvent(eventJson: String)
}

object NativeBridge {
    val lib: LodestoneLib by lazy {
        try {
            Native.load("lodestone_ffi", LodestoneLib::class.java)
        } catch (e: Throwable) {
            android.util.Log.e("Lodestone", "Failed to load native library", e)
            throw e
        }
    }

    fun engineNew(dataDir: String): Pointer = lib.lodestone_engine_new(dataDir)
    fun engineFree(engine: Pointer) = lib.lodestone_engine_free(engine)

    fun lastError(): String? = try {
        lib.lodestone_last_error()
    } catch (_: Throwable) {
        null
    }

    /** Reads the callee-allocated string out-param and frees it. */
    private fun readOut(rc: Int, out: PointerByReference): String? =
        if (rc == 0) {
            val p = out.value
            val s = p.getString(0)
            lib.lodestone_string_free(p)
            s
        } else null

    fun listServerTypes(engine: Pointer): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_server_types(engine, out), out)
    }

    fun deviceReport(engine: Pointer): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_device_report(engine, out), out)
    }

    fun listVersions(engine: Pointer, serverType: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_versions(engine, serverType, out), out)
    }

    fun listInstances(engine: Pointer): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_instances(engine, out), out)
    }

    fun createInstance(engine: Pointer, specJson: String): Int {
        val out = PointerByReference()
        return lib.lodestone_create_instance(engine, specJson, out)
    }

    fun deleteInstance(engine: Pointer, id: String): Int = lib.lodestone_delete_instance(engine, id)

    fun start(engine: Pointer, id: String): Int = lib.lodestone_start(engine, id)

    fun stop(engine: Pointer, id: String): Int = lib.lodestone_stop(engine, id)

    fun sendConsole(engine: Pointer, id: String, line: String): Int = lib.lodestone_send_console(engine, id, line)

    fun analyzeCrash(engine: Pointer, id: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_analyze_crash(engine, id, out), out)
    }

    fun registerEventCallback(engine: Pointer, cb: EventCallback): Int =
        lib.lodestone_register_event_callback(engine, cb)

    fun startSleepProxy(
        engine: Pointer,
        configJson: String,
        bindPort: Int,
        serverHost: String,
        serverPort: Int
    ): Int = lib.lodestone_start_sleep_proxy(engine, configJson, bindPort, serverHost, serverPort)

    fun stopSleepProxy(engine: Pointer): Int = lib.lodestone_stop_sleep_proxy(engine)

    fun startRemoteApi(engine: Pointer, token: String, bindPort: Int, allowWan: Int): Int =
        lib.lodestone_start_remote_api(engine, token, bindPort, allowWan)

    fun stopRemoteApi(engine: Pointer): Int = lib.lodestone_stop_remote_api(engine)

    fun enableCrossplay(engine: Pointer, instanceId: String, optionsJson: String): Int =
        lib.lodestone_enable_crossplay(engine, instanceId, optionsJson)

    fun listFiles(engine: Pointer, rel: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_files(engine, rel, out), out)
    }

    fun readFile(engine: Pointer, rel: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_read_file(engine, rel, out), out)
    }

    fun writeFile(engine: Pointer, rel: String, content: String): Int =
        lib.lodestone_write_file(engine, rel, content)

    fun installContent(engine: Pointer, projectId: String, versionId: String, destDir: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_install_content(engine, projectId, versionId, destDir, out), out)
    }

    fun createBackup(engine: Pointer, instanceId: String, sourceDir: String, label: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_create_backup(engine, instanceId, sourceDir, label, out), out)
    }

    fun listBackups(engine: Pointer, instanceId: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_backups(engine, instanceId, out), out)
    }

    fun restoreBackup(engine: Pointer, archivePath: String, destDir: String): Int {
        val count = LongByReference()
        return lib.lodestone_restore_backup(engine, archivePath, destDir, count)
    }

    fun deleteBackup(engine: Pointer, archivePath: String): Int =
        lib.lodestone_delete_backup(engine, archivePath)

    fun searchContent(engine: Pointer, query: String, limit: Int = 10): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_search_content(engine, query, limit, out), out)
    }

    fun listContentVersions(engine: Pointer, projectId: String): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_content_versions(engine, projectId, out), out)
    }

    fun applyTuning(engine: Pointer, serverType: String, profile: String, workDir: String): Int =
        lib.lodestone_apply_tuning(engine, serverType, profile, workDir)
}
