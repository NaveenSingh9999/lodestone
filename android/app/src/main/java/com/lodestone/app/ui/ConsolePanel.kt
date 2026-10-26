package com.lodestone.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDropDown
import androidx.compose.material.icons.rounded.Clear
import androidx.compose.material.icons.rounded.Send
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.state.AppState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun ConsoleContent(model: AppModel) {
    val scope = rememberCoroutineScope()
    var lines by remember { mutableStateOf(AppState.consoleLines()) }
    var selected by remember { mutableStateOf("") }
    var menuOpen by remember { mutableStateOf(false) }
    var command by remember { mutableStateOf("") }

    val listener = remember { { _: String, _: String -> lines = AppState.consoleLines() } }
    DisposableEffect(Unit) {
        AppState.addListener(listener)
        onDispose { AppState.removeListener(listener) }
    }
    // Keep the buffer fresh when this screen is open but idle (poll + listener).
    LaunchedEffect(Unit) {
        while (true) {
            delay(400)
            lines = AppState.consoleLines()
        }
    }
    if (selected.isBlank()) {
        LaunchedEffect(model.instances) {
            if (selected.isBlank()) selected = model.instances.firstOrNull()?.id ?: ""
        }
    }

    val listState = rememberLazyListState()
    LaunchedEffect(lines.size) {
        if (lines.isNotEmpty()) listState.scrollToItem(lines.size - 1)
    }

    Column(modifier = Modifier.fillMaxSize().imePadding()) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedButton(onClick = { menuOpen = true }) {
                Text(if (selected.isBlank()) "No instance" else selected)
                Icon(Icons.Rounded.ArrowDropDown, contentDescription = "Choose instance")
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                if (model.instances.isEmpty()) {
                    DropdownMenuItem(
                        text = { Text("Create an instance first") },
                        onClick = { menuOpen = false },
                    )
                }
                model.instances.forEach { inst ->
                    DropdownMenuItem(
                        text = { Text("${inst.id} (${inst.state})") },
                        onClick = {
                            selected = inst.id
                            menuOpen = false
                        },
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            IconButton(onClick = {
                AppState.clearConsole()
                lines = emptyList()
            }) {
                Icon(Icons.Rounded.Clear, contentDescription = "Clear console")
            }
        }
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .padding(horizontal = 16.dp),
            shape = RoundedCornerShape(14.dp),
            color = MaterialTheme.colorScheme.surfaceVariant,
        ) {
            if (lines.isEmpty()) {
                Column(
                    modifier = Modifier.fillMaxSize(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Text(
                        "Server output appears here.\nCommands typed below are sent as console input.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(16.dp),
                    )
                }
            } else {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize().padding(10.dp),
                ) {
                    items(lines) { line ->
                        Text(
                            line,
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = command,
                onValueChange = { command = it },
                label = { Text("Console command") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            FilledIconButton(
                enabled = command.isNotBlank() && selected.isNotBlank(),
                onClick = {
                    val cmd = command.trim()
                    val id = selected
                    scope.launch {
                        model.withBusy {
                            val err = withContext(Dispatchers.IO) { model.sendConsoleNow(id, cmd) }
                            if (err == null) {
                                AppState.appendConsole("› $cmd")
                                command = ""
                            } else {
                                model.toast = err
                            }
                        }
                    }
                },
            ) {
                Icon(Icons.Rounded.Send, contentDescription = "Send")
            }
        }
    }
}
