package ru.burmalda.journal

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

// Показ локальных пушей о новых оценках.
// Вызывается из Rust через JNI — как из UI-процесса, так и из GradesWorker,
// поэтому здесь не должно быть ничего, что требует Activity (кроме
// requestPermission).
object Notifier {
    private const val CHANNEL_ID = "grades_channel"

    @JvmStatic
    fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT >= 26) {
            val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (nm.getNotificationChannel(CHANNEL_ID) == null) {
                val ch = NotificationChannel(
                    CHANNEL_ID,
                    "Оценки",
                    NotificationManager.IMPORTANCE_HIGH
                )
                ch.description = "Уведомления о новых оценках"
                nm.createNotificationChannel(ch)
            }
        }
    }

    // Показать уведомление. На Android 13+ молча ничего не делает, если
    // пользователь не дал разрешение POST_NOTIFICATIONS.
    @JvmStatic
    fun notify(context: Context, id: Int, title: String, text: String) {
        val app = context.applicationContext
        if (Build.VERSION.SDK_INT >= 33 &&
            app.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }

        ensureChannel(app)
        val nm = app.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(app, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(app)
        }

        // Adaptive-иконка лаунчера как small icon даёт серый квадрат (а на части
        // прошивок — краш); используем монохромную, лаунчерная — только фолбэк.
        val mono = app.resources.getIdentifier(
            "ic_launcher_monochrome", "mipmap", app.packageName
        )

        // Тап по пушу открывает приложение — из воркера уведомление приходит
        // при закрытом приложении, без этого оно "мёртвое".
        val launch = app.packageManager.getLaunchIntentForPackage(app.packageName)
        if (launch != null) {
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                if (Build.VERSION.SDK_INT >= 23) PendingIntent.FLAG_IMMUTABLE else 0
            builder.setContentIntent(PendingIntent.getActivity(app, id, launch, flags))
        }

        val notification = builder
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setSmallIcon(if (mono != 0) mono else app.applicationInfo.icon)
            .setAutoCancel(true)
            .build()

        nm.notify(id, notification)
    }

    // Запрос рантайм-разрешения на уведомления (Android 13+).
    @JvmStatic
    fun requestPermission(context: Context) {
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            var act: Activity? = null
            var ctx: Context? = context
            while (ctx is android.content.ContextWrapper) {
                if (ctx is Activity) {
                    act = ctx
                    break
                }
                ctx = ctx.baseContext
            }
            if (act == null && context is Activity) {
                act = context
            }
            try {
                act?.requestPermissions(
                    arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                    1001
                )
            } catch (e: Throwable) {
                android.util.Log.w("burmalda57", "requestPermissions failed: ${e.message}")
            }
        }
    }

    // Показ короткого тоста на UI-потоке
    @JvmStatic
    fun showToast(context: Context, text: String) {
        val mainHandler = android.os.Handler(android.os.Looper.getMainLooper())
        mainHandler.post {
            android.widget.Toast.makeText(context, text, android.widget.Toast.LENGTH_SHORT).show()
        }
    }
}
