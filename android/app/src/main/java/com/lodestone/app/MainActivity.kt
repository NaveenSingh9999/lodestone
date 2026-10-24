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
import com.lodestone.app.server.LodestoneService

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    HomeScreen(onStart = {
                        startForegroundService(Intent(this, LodestoneService::class.java))
                    })
                }
            }
        }
    }
}

@Composable
fun HomeScreen(onStart: () -> Unit) {
    var status by remember { mutableStateOf("idle") }
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Lodestone", style = MaterialTheme.typography.headlineMedium)
        Text(status, modifier = Modifier.padding(top = 8.dp))
        Button(onClick = { status = "starting"; onStart() }, modifier = Modifier.padding(top = 16.dp)) {
            Text("Start server")
        }
    }
}
