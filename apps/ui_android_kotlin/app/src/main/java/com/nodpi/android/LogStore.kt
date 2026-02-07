package com.nodpi.android

import java.util.concurrent.CopyOnWriteArraySet

object LogStore {
    private const val maxLines = 200
    private val lines = mutableListOf<String>()
    private var cursor = 0
    private val listeners = CopyOnWriteArraySet<(String) -> Unit>()
    private val cursorUpRegex = Regex("\u001B\\[(\\d+)A")
    private val ansiRegex = Regex("\u001B\\[[0-9;]*[A-Za-z]")

    fun append(line: String) {
        val snapshot = synchronized(lines) {
            cursor = lines.size
            writeLine(line)
            lines.joinToString("\n")
        }
        listeners.forEach { it(snapshot) }
    }

    fun appendRaw(raw: String) {
        val snapshot = synchronized(lines) {
            processRaw(raw)
            lines.joinToString("\n")
        }
        listeners.forEach { it(snapshot) }
    }

    fun get(): String {
        return synchronized(lines) { lines.joinToString("\n") }
    }

    fun clear() {
        val snapshot = synchronized(lines) {
            lines.clear()
            cursor = 0
            ""
        }
        listeners.forEach { it(snapshot) }
    }

    fun addListener(listener: (String) -> Unit) {
        listeners.add(listener)
    }

    fun removeListener(listener: (String) -> Unit) {
        listeners.remove(listener)
    }

    private fun processRaw(raw: String) {
        val parts = raw.split("\n")
        for (part in parts) {
            handleControl(part)
            val text = stripAnsi(part).trimEnd('\r')
            if (text.isNotBlank()) {
                writeLine(text)
            }
        }
    }

    private fun handleControl(part: String) {
        cursorUpRegex.findAll(part).forEach { match ->
            val value = match.groupValues.getOrNull(1)?.toIntOrNull() ?: 0
            if (value > 0) {
                cursor = (cursor - value).coerceAtLeast(0)
            }
        }
        if (part.contains("\u001B[2J")) {
            lines.clear()
            cursor = 0
        }
        if (part.contains("\u001B[H")) {
            cursor = 0
        }
    }

    private fun stripAnsi(text: String): String {
        return text.replace(ansiRegex, "")
    }

    private fun writeLine(line: String) {
        if (cursor < lines.size) {
            lines[cursor] = line
            cursor += 1
        } else {
            lines.add(line)
            cursor = lines.size
        }
        if (lines.size > maxLines) {
            val overflow = lines.size - maxLines
            repeat(overflow) { lines.removeAt(0) }
            cursor = (cursor - overflow).coerceAtLeast(0)
        }
    }
}
