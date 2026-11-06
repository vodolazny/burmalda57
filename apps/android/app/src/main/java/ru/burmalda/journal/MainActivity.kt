package ru.burmalda.journal

import android.app.NativeActivity
import android.content.res.Configuration
import android.graphics.Color
import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : NativeActivity() {

    private var backPressedTime: Long = 0
    private var darkTheme: Boolean? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val systemDark = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES
        if (darkTheme == null) updateSystemBars(systemDark)
    }

    // Called from Slint's thread through JNI; Window must be updated on Android's UI thread.
    fun updateSystemBars(dark: Boolean) {
        runOnUiThread {
            darkTheme = dark
            applySystemBarAppearance()
            // Reapply after NativeActivity attaches/configures its decor, using the latest theme.
            window.decorView.post {
                if (!isFinishing && !isDestroyed) applySystemBarAppearance()
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun applySystemBarAppearance() {
        val dark = darkTheme ?: return
        // Legacy translucent flags prevent light status bar icons on older Android versions.
        window.clearFlags(WindowManager.LayoutParams.FLAG_TRANSLUCENT_STATUS or
            WindowManager.LayoutParams.FLAG_TRANSLUCENT_NAVIGATION)
        window.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS)
        window.statusBarColor = Color.TRANSPARENT
        WindowInsetsControllerCompat(window, window.decorView).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) applySystemBarAppearance()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        nativeOnThemeChanged()
    }

    override fun onBackPressed() {
        // Проверяем, перехватил ли Rust/Slint жест назад (закрыл оверлей/переключил таб)
        try {
            if (nativeOnBackPressed()) {
                return
            }
        } catch (e: Throwable) {
            android.util.Log.e("burmalda57", "nativeOnBackPressed error: ${e.message}")
        }

        // Если мы на главном экране — двойной тап для выхода (таймер 2 секунды)
        val now = System.currentTimeMillis()
        if (now - backPressedTime < 2000) {
            super.onBackPressed()
        } else {
            backPressedTime = now
            Toast.makeText(this, "Нажмите «Назад» ещё раз для выхода", Toast.LENGTH_SHORT).show()
        }
    }

    private external fun nativeOnBackPressed(): Boolean
    private external fun nativeOnThemeChanged()

    companion object {
        init {
            System.loadLibrary("burmalda57")
        }
    }
}
