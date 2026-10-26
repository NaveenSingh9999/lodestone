package com.lodestone.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.lodestone.app.native.NativeBridge
import com.lodestone.app.server.LodestoneService
import com.sun.jna.Pointer

class MainActivity : ComponentActivity() {
    private var engine: Pointer? = null
    private var initError: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        try {
            engine = NativeBridge.engineNew(cacheDir.absolutePath)
        } catch (e: Exception) {
            android.util.Log.e("Lodestone", "Engine creation failed", e)
            initError = e.message ?: "Unknown error"
        }
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    MainScreen(
                        onStart = {
                            startForegroundService(Intent(this, LodestoneService::class.java))
                        },
                        engine = engine,
                        initError = initError
                    )
                }
            }
        }
    }

    override fun onDestroy() {
        engine?.let { NativeBridge.engineFree(it) }
        engine = null
        super.onDestroy()
    }
}

@Composable
fun MainScreen(onStart: () -> Unit, engine: Pointer?, initError: String?) {
    var showAccess by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text("Lodestone", style = MaterialTheme.typography.headlineMedium)
        if (initError != null) {
            Text("Init error: $initError", modifier = Modifier.padding(top = 8.dp))
        }
        if (showAccess) {
            AccessScreen(engine = engine)
        } else {
            HomeScreen(onStart = onStart)
        }
        Button(onClick = { showAccess = !showAccess }, modifier = Modifier.padding(top = 16.dp)) {
            Text(if (showAccess) "Back to Home" else "Access")
        }
    }
}

@Composable
fun HomeScreen(onStart: () -> Unit) {
    var status by remember { mutableStateOf("idle") }
    Column(
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(status, modifier = Modifier.padding(top = 8.dp))
        Button(onClick = { status = "starting"; onStart() }, modifier = Modifier.padding(top = 16.dp)) {
            Text("Start server")
        }
    }
}
