package com.lodestone.app.state

import com.sun.jna.Pointer

object EngineHolder {
    var engine: Pointer? = null
    var initError: String? = null
}