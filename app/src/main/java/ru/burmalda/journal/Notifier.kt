package ru.burmalda.journal

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

// Показ локальных пушей о новых оценках. Вызывается из Rust через JNI.
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
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }

        ensureChannel(context)
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(context, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(context)
        }

        val notification = builder
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setSmallIcon(context.applicationInfo.icon)
            .setAutoCancel(true)
            .build()

        nm.notify(id, notification)
    }

    // Запрос рантайм-разрешения на уведомления (Android 13+). Вызывать с Activity.
    @JvmStatic
    fun requestPermission(activity: Activity) {
        if (Build.VERSION.SDK_INT >= 33 &&
            activity.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            activity.requestPermissions(
                arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                1001
            )
        }
    }
}
