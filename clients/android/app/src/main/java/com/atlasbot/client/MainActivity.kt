package com.atlasbot.client

import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.atlasbot.client.auth.AuthSession
import com.atlasbot.client.auth.MOBILE_CALLBACK_SCHEME
import com.atlasbot.client.ui.AtlasBotApp

class MainActivity : ComponentActivity() {
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AuthSession.obtain(this)
        enableEdgeToEdge()
        setContent { AtlasBotApp() }
        handleAuthIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleAuthIntent(intent)
    }

    private fun handleAuthIntent(intent: Intent?) {
        val data = intent?.data ?: return
        if (data.scheme != MOBILE_CALLBACK_SCHEME) return
        val session = AuthSession.obtain(this)
        session.handleCallbackUri(data) { result ->
            mainHandler.post {
                result.onSuccess {
                    Toast.makeText(this, "Login ok — ticket stored", Toast.LENGTH_SHORT).show()
                }.onFailure { e ->
                    Toast.makeText(this, "Login failed: ${e.message}", Toast.LENGTH_LONG).show()
                }
            }
        }
    }
}
