package com.nodpi.android

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.google.android.material.snackbar.Snackbar
import com.google.android.material.R as MaterialR
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

class MainActivity : AppCompatActivity() {
    private lateinit var statusText: TextView
    private lateinit var inputHost: EditText
    private lateinit var inputPort: EditText
    private lateinit var inputBlacklistFile: EditText
    private lateinit var inputOutHost: EditText
    private lateinit var inputLogAccess: EditText
    private lateinit var inputLogError: EditText
    private lateinit var inputBlacklist: EditText
    private lateinit var spinnerFragment: Spinner
    private lateinit var spinnerDomain: Spinner
    private lateinit var checkboxNoBlacklist: CheckBox
    private lateinit var checkboxAutoBlacklist: CheckBox
    private lateinit var checkboxQuiet: CheckBox
    private lateinit var debugJniLibs: TextView
    private lateinit var buttonCopyDebug: Button
    private lateinit var debugLogs: TextView
    private lateinit var buttonClearLogs: Button
    private val logListener: (String) -> Unit = { text ->
        runOnUiThread {
            debugLogs.text = if (text.isBlank()) "(no logs yet)" else text
        }
    }

    private lateinit var configStore: ConfigStore
    private lateinit var blacklistStore: BlacklistStore
    private lateinit var rootDir: File

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        rootDir = File(filesDir, "nodpi")
        rootDir.mkdirs()

        configStore = ConfigStore(rootDir)
        blacklistStore = BlacklistStore(rootDir, configStore, AssetInstaller(this, rootDir, File(codeCacheDir, "nodpi_bin")))

        bindViews()
        setupSpinners()
        setupButtons()
        setupMutualExclusion()
        LogStore.addListener(logListener)
        loadState()
    }

    override fun onDestroy() {
        LogStore.removeListener(logListener)
        super.onDestroy()
    }

    private fun bindViews() {
        statusText = findViewById(R.id.status_text)
        inputHost = findViewById(R.id.input_host)
        inputPort = findViewById(R.id.input_port)
        inputBlacklistFile = findViewById(R.id.input_blacklist_file)
        inputOutHost = findViewById(R.id.input_out_host)
        inputLogAccess = findViewById(R.id.input_log_access)
        inputLogError = findViewById(R.id.input_log_error)
        inputBlacklist = findViewById(R.id.input_blacklist)
        spinnerFragment = findViewById(R.id.spinner_fragment)
        spinnerDomain = findViewById(R.id.spinner_domain)
        checkboxNoBlacklist = findViewById(R.id.checkbox_no_blacklist)
        checkboxAutoBlacklist = findViewById(R.id.checkbox_auto_blacklist)
        checkboxQuiet = findViewById(R.id.checkbox_quiet)
        debugJniLibs = findViewById(R.id.debug_jnilibs)
        buttonCopyDebug = findViewById(R.id.button_copy_debug)
        debugLogs = findViewById(R.id.debug_logs)
        buttonClearLogs = findViewById(R.id.button_clear_logs)
    }

    private fun setupSpinners() {
        val fragmentItems = listOf("random", "sni")
        val domainItems = listOf("strict", "loose")

        spinnerFragment.adapter = ArrayAdapter(
            this,
            android.R.layout.simple_spinner_dropdown_item,
            fragmentItems
        )
        spinnerDomain.adapter = ArrayAdapter(
            this,
            android.R.layout.simple_spinner_dropdown_item,
            domainItems
        )
    }

    private fun setupButtons() {
        findViewById<Button>(R.id.button_refresh).setOnClickListener {
            refreshStatus()
        }
        findViewById<Button>(R.id.button_save_config).setOnClickListener {
            saveConfig(showMessage = true)
        }
        findViewById<Button>(R.id.button_save_blacklist).setOnClickListener {
            saveBlacklist(showMessage = true)
        }
        findViewById<Button>(R.id.button_start).setOnClickListener {
            lifecycleScope.launch {
                saveConfig(showMessage = false)
                saveBlacklist(showMessage = false)
                withContext(Dispatchers.IO) {
                    ServerService.start(this@MainActivity)
                }
                showMessage("Server starting")
                refreshStatus()
            }
        }
        findViewById<Button>(R.id.button_stop).setOnClickListener {
            lifecycleScope.launch {
                withContext(Dispatchers.IO) {
                    ServerService.stop(this@MainActivity)
                }
                showMessage("Server stopping")
                refreshStatus()
            }
        }
        findViewById<Button>(R.id.button_restart).setOnClickListener {
            lifecycleScope.launch {
                saveConfig(showMessage = false)
                saveBlacklist(showMessage = false)
                withContext(Dispatchers.IO) {
                    ServerService.stop(this@MainActivity)
                }
                delay(400)
                withContext(Dispatchers.IO) {
                    ServerService.start(this@MainActivity)
                }
                showMessage("Server restarting")
                refreshStatus()
            }
        }
        buttonCopyDebug.setOnClickListener {
            val text = buildString {
                append(debugJniLibs.text.toString())
                append('\n')
                append(debugLogs.text.toString())
            }
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("nodpi-debug", text))
            Snackbar.make(findViewById(R.id.footer_hint), "Debug copied", Snackbar.LENGTH_SHORT).show()
        }
        buttonClearLogs.setOnClickListener {
            LogStore.clear()
            Snackbar.make(findViewById(R.id.footer_hint), "Logs cleared", Snackbar.LENGTH_SHORT).show()
        }
    }


    private fun setupMutualExclusion() {
        checkboxNoBlacklist.setOnCheckedChangeListener { _, isChecked ->
            if (isChecked) checkboxAutoBlacklist.isChecked = false
        }
        checkboxAutoBlacklist.setOnCheckedChangeListener { _, isChecked ->
            if (isChecked) checkboxNoBlacklist.isChecked = false
        }
    }

    private fun loadState() {
        lifecycleScope.launch {
            val config = withContext(Dispatchers.IO) { configStore.load() }
            applyConfigToUi(config)
            val blacklist = withContext(Dispatchers.IO) { blacklistStore.load(config) }
            inputBlacklist.setText(blacklist)
            updateDebugTree()
            debugLogs.text = LogStore.get().ifBlank { "(no logs yet)" }
            refreshStatus()
        }
    }

    private fun updateDebugTree() {
        val nativeDir = File(applicationInfo.nativeLibraryDir)
        val nativeTree = buildTree(nativeDir, "nativeLibraryDir")
        debugJniLibs.text = buildString {
            append(nativeTree)
        }
    }

    private fun buildTree(root: File, label: String): String {
        val lines = mutableListOf<String>()
        lines += "$label: ${root.absolutePath}"
        if (!root.exists()) {
            lines += "  (missing)"
            return lines.joinToString("\n")
        }
        val entries = root.listFiles()?.sortedBy { it.name } ?: emptyList<File>()
        if (entries.isEmpty()) {
            lines += "  (empty)"
            return lines.joinToString("\n")
        }
        lines += entries.map { entry ->
            val marker = if (entry.isDirectory) "/" else ""
            "  ${entry.name}$marker"
        }
        return lines.joinToString("\n")
    }

    private fun refreshStatus() {
        val running = ServerService.isRunning()
        val status = if (running) "Running" else "Stopped"
        statusText.text = status
    }

    private fun applyConfigToUi(config: ProxyConfig) {
        inputHost.setText(config.host)
        inputPort.setText(config.port.toString())
        inputBlacklistFile.setText(config.blacklistFile)
        inputOutHost.setText(config.outHost.orEmpty())
        inputLogAccess.setText(config.logAccessFile.orEmpty())
        inputLogError.setText(config.logErrorFile.orEmpty())
        checkboxNoBlacklist.isChecked = config.noBlacklist
        checkboxAutoBlacklist.isChecked = config.autoBlacklist
        checkboxQuiet.isChecked = config.quiet

        setSpinnerValue(spinnerFragment, config.fragmentMethod)
        setSpinnerValue(spinnerDomain, config.domainMatching)
    }

    private fun setSpinnerValue(spinner: Spinner, value: String) {
        for (i in 0 until spinner.count) {
            if (spinner.getItemAtPosition(i).toString() == value) {
                spinner.setSelection(i)
                return
            }
        }
    }

    private fun readConfigFromUi(): ProxyConfig {
        val port = inputPort.text.toString().toIntOrNull() ?: 8881
        return ProxyConfig(
            host = inputHost.text.toString().ifBlank { "0.0.0.0" },
            port = port,
            blacklistFile = inputBlacklistFile.text.toString().ifBlank { "blacklist.txt" },
            fragmentMethod = spinnerFragment.selectedItem.toString(),
            domainMatching = spinnerDomain.selectedItem.toString(),
            outHost = inputOutHost.text.toString().ifBlank { null },
            logAccessFile = inputLogAccess.text.toString().ifBlank { null },
            logErrorFile = inputLogError.text.toString().ifBlank { null },
            noBlacklist = checkboxNoBlacklist.isChecked,
            autoBlacklist = checkboxAutoBlacklist.isChecked,
            quiet = checkboxQuiet.isChecked
        )
    }

    private fun saveConfig(showMessage: Boolean) {
        lifecycleScope.launch {
            val config = readConfigFromUi()
            withContext(Dispatchers.IO) {
                configStore.save(config)
            }
            if (showMessage) showMessage("Config saved")
        }
    }

    private fun saveBlacklist(showMessage: Boolean) {
        lifecycleScope.launch {
            val config = readConfigFromUi()
            val text = inputBlacklist.text.toString()
            withContext(Dispatchers.IO) {
                blacklistStore.save(config, text)
            }
            if (showMessage) showMessage("Blacklist saved")
        }
    }

    private fun showMessage(message: String) {
        val snackbar = Snackbar.make(findViewById(R.id.footer_hint), message, Snackbar.LENGTH_INDEFINITE)
        snackbar.setAction("Copy") {
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("nodpi-message", message))
            Snackbar.make(findViewById(R.id.footer_hint), "Copied", Snackbar.LENGTH_SHORT).show()
        }
        val textView = snackbar.view.findViewById<TextView>(MaterialR.id.snackbar_text)
        textView.maxLines = 6
        textView.textSize = 14f
        snackbar.show()
    }
}
