package com.lodestone.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Save
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.lodestone.app.AppModel
import com.lodestone.app.FileEntry
import com.lodestone.app.fmtBytes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

internal fun parentPath(rel: String): String {
    if (rel.isEmpty()) return ""
    return rel.substringBeforeLast('/', "")
}

internal fun joinPath(parent: String, name: String): String =
    if (parent.isEmpty()) name else "$parent/$name"

internal fun displayPath(rel: String): String = if (rel.isEmpty()) "/" else "/$rel"

@Composable
fun FilesContent(model: AppModel) {
    val scope = rememberCoroutineScope()
    var rel by remember { mutableStateOf("") }
    var reload by remember { mutableStateOf(0) }
    var entries by remember { mutableStateOf<List<FileEntry>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var editing by remember { mutableStateOf<Pair<String, String>?>(null) }

    LaunchedEffect(rel, reload) {
        model.withBusy {
            val (list, msg): Pair<List<FileEntry>?, String?> =
                withContext(Dispatchers.IO) { model.listFilesNow(rel) }
            entries = list
            error = msg
            if (msg != null) model.toast = msg
        }
    }

    val edit = editing
    if (edit != null) {
        EditorScreen(
            rel = edit.first,
            initial = edit.second,
            onClose = { editing = null },
            onSave = { text ->
                editing = null
                scope.launch {
                    model.withBusy {
                        val err = withContext(Dispatchers.IO) { model.writeFileNow(edit.first, text) }
                        model.toast = err ?: "Saved ${edit.first}"
                        if (err == null) reload++
                    }
                }
            },
        )
        return
    }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(
                enabled = rel.isNotEmpty(),
                onClick = { rel = parentPath(rel) },
            ) { Icon(Icons.Rounded.ArrowBack, contentDescription = "Parent folder") }
            Column(modifier = Modifier.weight(1f)) {
                Text(displayPath(rel), style = MaterialTheme.typography.titleMedium, maxLines = 1)
                Text(
                    "root: ${model.filesRoot.name}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            IconButton(onClick = { reload++ }) {
                Icon(Icons.Rounded.Refresh, contentDescription = "Refresh")
            }
        }
        HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))

        if (error != null) {
            Column(modifier = Modifier.padding(16.dp)) {
                EmptyHint("Can't list ${displayPath(rel)}:\n${error}")
            }
        }
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(8.dp, 8.dp, 8.dp, 16.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            val list = entries
            if (list != null && list.isEmpty() && error == null) {
                item { EmptyHint("Empty folder") }
            }
            if (list != null) {
                items(list.size) { i ->
                    val entry = list[i]
                    FileRow(entry) {
                        val child = joinPath(rel, entry.name)
                        if (entry.isDir) {
                            rel = child
                        } else {
                            scope.launch {
                                model.withBusy {
                                    val result: Pair<String?, String?> =
                                        withContext(Dispatchers.IO) { model.readFileNow(child) }
                                    val err: String? = result.second as? String
                                    if (err != null) {
                                        model.toast = err
                                    } else {
                                        editing = child to (result.first as? String ?: "")
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun FileRow(entry: FileEntry, onOpen: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onOpen() }
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (entry.isDir) Icons.Rounded.Folder else Icons.Rounded.Description,
            contentDescription = if (entry.isDir) "Folder" else "File",
            tint = if (entry.isDir) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(modifier = Modifier.weight(1f).padding(start = 12.dp)) {
            Text(entry.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
        }
        if (!entry.isDir) {
            Text(
                fmtBytes(entry.size),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun EditorScreen(
    rel: String,
    initial: String,
    onClose: () -> Unit,
    onSave: (String) -> Unit,
) {
    var text by remember { mutableStateOf(initial) }
    Column(modifier = Modifier.fillMaxSize().imePadding()) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onClose) {
                Icon(Icons.Rounded.ArrowBack, contentDescription = "Close editor")
            }
            Text(
                rel,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { onSave(text) }) {
                Icon(Icons.Rounded.Save, contentDescription = "Save file")
            }
        }
        HorizontalDivider(modifier = Modifier.padding(horizontal = 8.dp))
        OutlinedTextField(
            value = text,
            onValueChange = { text = it },
            modifier = Modifier.fillMaxSize().padding(8.dp),
            textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            shape = RoundedCornerShape(12.dp),
        )
    }
}
