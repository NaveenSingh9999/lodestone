package com.lodestone.app.tile

import android.content.Intent
import android.service.quicksettings.TileService
import com.lodestone.app.server.LodestoneService
import com.lodestone.app.state.StateStore

class QsTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        val tile = qsTile
        tile.state = if (StateStore.anyRunning()) {
            android.service.quicksettings.Tile.STATE_ACTIVE
        } else {
            android.service.quicksettings.Tile.STATE_INACTIVE
        }
        tile.label = if (StateStore.anyRunning()) "Lodestone: Running" else "Lodestone: Stopped"
        tile.updateTile()
    }

    override fun onClick() {
        super.onClick()
        val running = StateStore.anyRunning()
        val intent = Intent(applicationContext, LodestoneService::class.java).apply {
            action = if (running) LodestoneService.ACTION_STOP_ALL else LodestoneService.ACTION_START_ALL
        }
        try {
            applicationContext.startForegroundService(intent)
        } catch (e: Exception) {
            applicationContext.startService(intent)
        }
    }
}