package com.lodestone.app.native

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.ptr.PointerByReference

interface LodestoneLib : Library {
    fun lodestone_engine_new(dataDir: String): Pointer
    fun lodestone_engine_free(engine: Pointer)
    fun lodestone_string_free(s: Pointer)
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
    fun lodestone_start_sleep_proxy(engine: Pointer, configJson: String, bindPort: Int, serverHost: String, serverPort: Int): Int
    fun lodestone_stop_sleep_proxy(engine: Pointer): Int
    fun lodestone_start_remote_api(engine: Pointer, token: String, bindPort: Int, allowWan: Int): Int
    fun lodestone_stop_remote_api(engine: Pointer): Int
    fun lodestone_enable_crossplay(engine: Pointer, instanceId: String, optionsJson: String): Int
}

fun interface EventCallback {
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
    fun readOut(rc: Int, out: PointerByReference): String? =
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

    fun listInstances(engine: Pointer): String? {
        val out = PointerByReference()
        return readOut(lib.lodestone_list_instances(engine, out), out)
    }

    fun createInstance(engine: Pointer, specJson: String): Int {
        val out = PointerByReference()
        val rc = lib.lodestone_create_instance(engine, specJson, out)
        if (rc == 0) {
            val p = out.value
            lib.lodestone_string_free(p)
        }
        return rc
    }

    fun deleteInstance(engine: Pointer, id: String): Int = lib.lodestone_delete_instance(engine, id)

    fun start(engine: Pointer, id: String): Int = lib.lodestone_start(engine, id)

    fun stop(engine: Pointer, id: String): Int = lib.lodestone_stop(engine, id)

    fun sendConsole(engine: Pointer, id: String, line: String): Int = lib.lodestone_send_console(engine, id, line)

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

    fun startRemoteApi(
        engine: Pointer,
        token: String,
        bindPort: Int,
        allowWan: Int
    ): Int = lib.lodestone_start_remote_api(engine, token, bindPort, allowWan)

    fun stopRemoteApi(engine: Pointer): Int = lib.lodestone_stop_remote_api(engine)

    fun enableCrossplay(
        engine: Pointer,
        instanceId: String,
        optionsJson: String
    ): Int = lib.lodestone_enable_crossplay(engine, instanceId, optionsJson)
}
