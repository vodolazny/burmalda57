package ru.burmalda.journal

import android.app.NativeActivity
import android.os.Bundle
import android.widget.Toast

class MainActivity : NativeActivity() {

    private var backPressedTime: Long = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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

    companion object {
        init {
            System.loadLibrary("burmalda57")
        }
    }
}
