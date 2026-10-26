package com.lodestone.app.widget

import android.content.Context
import androidx.glance.GlanceId
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.provideContent
import androidx.glance.layout.Column
import androidx.glance.text.Text
import com.lodestone.app.state.StateStore

class LodestoneWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            val states = StateStore.snapshot()
            val summary = if (states.isEmpty()) "No instances" else states.entries.joinToString(", ") { "${it.key}=${it.value}" }
            Column {
                Text(text = "Lodestone Server")
                Text(text = summary)
            }
        }
    }
}