package com.lodestone.app.widget

import android.content.Context
import android.content.Intent
import androidx.glance.action.ActionCallback
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceId
import androidx.glance.modifier.GlanceModifier
import androidx.glance.widget.Button
import androidx.glance.widget.Column
import androidx.glance.widget.Row
import androidx.glance.widget.Text
import com.lodestone.app.server.LodestoneService
import com.lodestone.app.state.StateStore

class LodestoneWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId): GlanceModifier {
        val states = StateStore.snapshot()
        val summary = if (states.isEmpty()) "No instances" else states.entries.joinToString(", ") { "${it.key}=${it.value}" }

        val startAction = ActionCallback {
            context.startForegroundService(
                Intent(context, LodestoneService::class.java).apply {
                    action = LodestoneService.ACTION_START_ALL
                }
            )
        }
        val stopAction = ActionCallback {
            context.startForegroundService(
                Intent(context, LodestoneService::class.java).apply {
                    action = LodestoneService.ACTION_STOP_ALL
                }
            )
        }

        return Column {
            Text(text = "Lodestone Server")
            Text(text = summary)
            Row {
                Button(onClick = startAction) {
                    Text(text = "Start")
                }
                Button(onClick = stopAction) {
                    Text(text = "Stop")
                }
            }
        }
    }
}
