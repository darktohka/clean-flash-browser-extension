package org.cleanflash.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.activity.result.contract.ActivityResultContracts

/**
 * Main launcher activity — lets users enter a SWF URL or pick a local file.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var urlInput: EditText
    private lateinit var statusText: TextView

    private val filePickerLauncher = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri: Uri? ->
        if (uri != null) {
            launchPlayer(uri.toString(), isLocalFile = true)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        urlInput = findViewById(R.id.url_input)
        statusText = findViewById(R.id.status_text)

        findViewById<Button>(R.id.btn_open_url).setOnClickListener {
            val url = urlInput.text.toString().trim()
            if (url.isNotEmpty()) {
                launchPlayer(url, isLocalFile = false)
            } else {
                Toast.makeText(this, "Please enter a URL", Toast.LENGTH_SHORT).show()
            }
        }

        findViewById<Button>(R.id.btn_open_file).setOnClickListener {
            filePickerLauncher.launch(arrayOf("application/x-shockwave-flash", "*/*"))
        }

        findViewById<Button>(R.id.btn_settings).setOnClickListener {
            startActivity(Intent(this, SettingsActivity::class.java))
        }

        // Check container status
        val container = ContainerManager(this)
        if (container.isInitialized) {
            statusText.text = "Ready"
        } else {
            statusText.text = "First-run setup required. Container will be initialized on first play."
        }

        // Handle intent if launched with a SWF file
        handleIncomingIntent(intent)
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        intent?.let { handleIncomingIntent(it) }
    }

    private fun handleIncomingIntent(intent: Intent) {
        val uri = intent.data ?: return
        val scheme = uri.scheme ?: return

        when (scheme) {
            "http", "https" -> launchPlayer(uri.toString(), isLocalFile = false)
            "content", "file" -> launchPlayer(uri.toString(), isLocalFile = true)
        }
    }

    private fun launchPlayer(source: String, isLocalFile: Boolean) {
        val intent = Intent(this, FlashPlayerActivity::class.java).apply {
            putExtra(FlashPlayerActivity.EXTRA_SWF_SOURCE, source)
            putExtra(FlashPlayerActivity.EXTRA_IS_LOCAL_FILE, isLocalFile)
        }
        startActivity(intent)
    }
}
