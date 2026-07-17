package ru.burmalda.journal

import android.annotation.SuppressLint
import android.app.Activity
import android.os.Bundle
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

class EsiaAuthActivity : Activity() {

    private var tokenSent = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        com.burmalda57.crypto.KeystoreCrypto.init(applicationContext)

        val webView = WebView(this)
        setContentView(webView)

        val cookieManager = CookieManager.getInstance()
        cookieManager.setAcceptCookie(true)
        cookieManager.setAcceptThirdPartyCookies(webView, true)

        webView.settings.javaScriptEnabled = true
        webView.settings.domStorageEnabled = true

        webView.webViewClient = object : WebViewClient() {

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                checkCookiesForToken(url)
            }

            override fun shouldOverrideUrlLoading(
                view: WebView?,
                request: WebResourceRequest?
            ): Boolean {
                val url = request?.url?.toString() ?: return false
                // Проверяем куки при редиректе на домен дневника
                if (url.contains("obr57.ru") || url.contains("shkolove.ru")) {
                    checkCookiesForToken(url)
                }
                return false // Позволяем WebView самому обрабатывать переходы
            }
        }

        webView.loadUrl("https://passport.obr57.ru/auth/esia/redirect/?returnTo=https://one.obr57.ru")
    }

    private fun checkCookiesForToken(url: String?) {
        if (url == null || tokenSent) return

        val cookies = CookieManager.getInstance().getCookie(url) ?: return

        cookies.split(";").forEach { cookie ->
            val trimmed = cookie.trim()
            if (trimmed.startsWith("X1_SSO=")) {
                val token = trimmed.removePrefix("X1_SSO=").trim()
                if (token.isNotEmpty() && token != "deleted") {
                    tokenSent = true

                    // Путь к приватному хранилищу приложения (туда ляжет .session)
                    val storagePath = filesDir.absolutePath

                    sendTokenToRust(token, storagePath)
                    finish()
                    return
                }
            }
        }
    }

    private external fun sendTokenToRust(
        token: String,
        storagePath: String
    )

    companion object {
        init {
            System.loadLibrary("burmalda57")
        }
    }
}