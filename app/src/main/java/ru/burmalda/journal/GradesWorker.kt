package ru.burmalda.journal

import android.content.Context
import android.util.Log
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

// Периодический опрос новых оценок через WorkManager.
//
// Вся логика (сеть, снимок, дифф, пуш) осталась в Rust (notify.rs) — воркер
// только даёт запуск системным планировщиком: работает, даже когда процесс
// приложения убит или устройство перезагружено.
//
// nativePoll возвращает: 0 — успех (в т.ч. «делать нечего»), 1 — временная
// ошибка (сеть/блокировка) → Result.retry().
class GradesWorker(appContext: Context, params: WorkerParameters) :
    Worker(appContext, params) {

    override fun doWork(): Result {
        return try {
            when (nativePoll(applicationContext, applicationContext.filesDir.absolutePath)) {
                0 -> Result.success()
                else -> if (runAttemptCount >= MAX_ATTEMPTS) Result.success() else Result.retry()
            }
        } catch (t: Throwable) {
            // Исключение наружу выпускать нельзя: работа стала бы FAILED и
            // периодика больше не запустилась бы никогда.
            Log.w(TAG, "grades poll failed: ${t.message}", t)
            if (runAttemptCount >= MAX_ATTEMPTS) Result.success() else Result.retry()
        }
    }

    companion object {
        private const val TAG = "burmalda57"
        private const val UNIQUE_NAME = "grades_poll"
        private const val MAX_ATTEMPTS = 3

        // Имя .so из rust/Cargo.toml ([package] name = "burmalda57").
        private const val NATIVE_LIB = "burmalda57"

        // Воркер может стартовать в свежем процессе, где MainActivity не
        // запускалась и .so ещё не загружена. loadLibrary идемпотентен.
        init {
            try {
                System.loadLibrary(NATIVE_LIB)
            } catch (t: Throwable) {
                Log.e(TAG, "loadLibrary($NATIVE_LIB) failed: ${t.message}", t)
            }
        }

        @JvmStatic
        external fun nativePoll(context: Context, filesDir: String): Int

        // Минимальный период WorkManager — 15 минут; точный момент запуска
        // выбирает система (Doze / оптимизация батареи).
        @JvmStatic
        @JvmOverloads
        fun schedule(context: Context, intervalMinutes: Long = 30L) {
            val interval = if (intervalMinutes < 15L) 15L else intervalMinutes

            val request = PeriodicWorkRequestBuilder<GradesWorker>(
                interval, TimeUnit.MINUTES,
                10, TimeUnit.MINUTES // flex-окно
            )
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .build()
                )
                .setBackoffCriteria(BackoffPolicy.LINEAR, 5, TimeUnit.MINUTES)
                .setInitialDelay(1, TimeUnit.MINUTES)
                .addTag(UNIQUE_NAME)
                .build()

            WorkManager.getInstance(context.applicationContext)
                .enqueueUniquePeriodicWork(
                    UNIQUE_NAME,
                    // UPDATE — применить новые период/ограничения без сброса
                    // расписания на каждом запуске приложения.
                    ExistingPeriodicWorkPolicy.UPDATE,
                    request
                )
        }

        @JvmStatic
        fun cancel(context: Context) {
            WorkManager.getInstance(context.applicationContext)
                .cancelUniqueWork(UNIQUE_NAME)
        }
    }
}
