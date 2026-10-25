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
    fun lodestone_list_instances(engine: Pointer, out: PointerByReference): Int
    fun lodestone_start_sleep_proxy(engine: Pointer, configJson: String, bindPort: Int, serverHost: String, serverPort: Int): Int
    fun lodestone_stop_sleep_proxy(engine: Pointer): Int
    fun lodestone_start_remote_api(engine: Pointer, token: String, bindPort: Int, allowWan: Int): Int
    fun lodestone_stop_remote_api(engine: Pointer): Int
}

fun interface EventCallback {
    fun onEvent(eventJson: String)
}

object NativeBridge {
    val lib: LodestoneLib = Native.load("lodestone_ffi", LodestoneLib::class.java)

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
}
