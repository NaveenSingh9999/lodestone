package com.lodestone.app.widget

import android.content.Context
import android.content.Intent
import androidx.glance.GlanceId
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.layout.Column
import androidx.glance.text.Text
import com.lodestone.app.state.StateStore

class LodestoneWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId): androidx.glance.GlanceModifier {
        val states = StateStore.snapshot()
        val summary = if (states.isEmpty()) "No instances" else states.entries.joinToString(", ") { "${it.key}=${it.value}" }

        return androidx.glance.layout.Column {
            androidx.glance.text.Text(text = "Lodestone Server")
            androidx.glance.text.Text(text = summary)
        }
    }
}

class LodestoneWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = LodestoneWidget()
}