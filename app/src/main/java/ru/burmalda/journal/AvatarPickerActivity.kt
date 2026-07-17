package ru.burmalda.journal

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import java.io.ByteArrayOutputStream
import kotlin.math.max

/**
 * Прозрачная активити для выбора аватара.
 *
 * Открывает системный пикер картинок, ужимает выбранное до MAX_SIZE
 * по большей стороне, кодирует в JPEG и отдаёт байты в Rust через
 * nativeSetAvatar(). Rust сам сохраняет файл и обновляет UI.
 */
class AvatarPickerActivity : Activity() {

    companion object {
        private const val REQ_PICK = 1001
        private const val MAX_SIZE = 512

        init {
            // ИМЯ ДОЛЖНО СОВПАДАТЬ с нативной библиотекой, которую грузит основная Activity.
            // Если она уже загружена в этом процессе — повторная попытка безопасна.
            try { System.loadLibrary("burmalda57") } catch (_: Throwable) {}
        }

        @JvmStatic
        external fun nativeSetAvatar(bytes: ByteArray)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pick = Intent(Intent.ACTION_GET_CONTENT).apply {
            type = "image/*"
            addCategory(Intent.CATEGORY_OPENABLE)
        }
        try {
            startActivityForResult(Intent.createChooser(pick, "Выберите аватар"), REQ_PICK)
        } catch (e: Exception) {
            finish()
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_PICK && resultCode == RESULT_OK) {
            data?.data?.let { uri ->
                try {
                    loadDownscaledJpeg(uri)?.let { nativeSetAvatar(it) }
                } catch (_: Exception) {
                    // тихо игнорируем — аватар просто не поменяется
                }
            }
        }
        finish()
    }

    /** Загрузить картинку, уменьшить до MAX_SIZE и закодировать в JPEG. */
    private fun loadDownscaledJpeg(uri: Uri): ByteArray? {
        // 1) узнаём размеры без декодирования
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        contentResolver.openInputStream(uri)?.use {
            BitmapFactory.decodeStream(it, null, bounds)
        }
        val w = bounds.outWidth
        val h = bounds.outHeight
        if (w <= 0 || h <= 0) return null

        // 2) грубое уменьшение через inSampleSize
        var sample = 1
        while (max(w, h) / sample > MAX_SIZE * 2) sample *= 2
        val opts = BitmapFactory.Options().apply { inSampleSize = sample }
        val bmp = contentResolver.openInputStream(uri)?.use {
            BitmapFactory.decodeStream(it, null, opts)
        } ?: return null

        // 3) точное масштабирование до MAX_SIZE по большей стороне
        val scale = MAX_SIZE.toFloat() / max(bmp.width, bmp.height).toFloat()
        val scaled = if (scale < 1f) {
            Bitmap.createScaledBitmap(
                bmp,
                (bmp.width * scale).toInt().coerceAtLeast(1),
                (bmp.height * scale).toInt().coerceAtLeast(1),
                true
            )
        } else {
            bmp
        }

        // 4) JPEG
        val out = ByteArrayOutputStream()
        scaled.compress(Bitmap.CompressFormat.JPEG, 90, out)
        return out.toByteArray()
    }
}
