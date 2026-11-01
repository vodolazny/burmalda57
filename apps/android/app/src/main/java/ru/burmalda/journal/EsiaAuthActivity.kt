package ru.burmalda.journal

import android.annotation.SuppressLint
import android.app.Activity
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.net.http.SslError
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.webkit.ConsoleMessage
import android.webkit.CookieManager
import android.webkit.SslErrorHandler
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

class EsiaAuthActivity : Activity() {

    private var tokenSent = false
    private val TAG = "EsiaWebView"

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KeystoreCrypto.init(applicationContext)

        val isDebuggable = 0 != (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE)
        if (isDebuggable) {
            WebView.setWebContentsDebuggingEnabled(true)
        }

        val webView = WebView(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            webView.isForceDarkAllowed = false
        }
        setContentView(webView)

        val cookieManager = CookieManager.getInstance()
        cookieManager.setAcceptCookie(true)
        cookieManager.setAcceptThirdPartyCookies(webView, true)

        webView.settings.javaScriptEnabled = true
        webView.settings.domStorageEnabled = true

        if (isDebuggable) {
            webView.webChromeClient = object : WebChromeClient() {
                override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean {
                    consoleMessage?.let {
                        Log.d(
                            TAG,
                            "[JS Console] ${it.message()} (line ${it.lineNumber()} of ${it.sourceId()})"
                        )
                    }
                    return true
                }
            }
        }

        webView.webViewClient = object : WebViewClient() {

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                if (isDebuggable) Log.d(TAG, "Page finished: $url")
                checkCookiesForToken(url)
            }

            override fun onReceivedSslError(
                view: WebView?,
                handler: SslErrorHandler?,
                error: SslError?
            ) {
                if (isDebuggable) {
                    val cert = error?.certificate
                    Log.w(
                        TAG,
                        "SSL Error: ${error?.primaryError} on URL: ${error?.url}\n" +
                            "Issued to: ${cert?.issuedTo?.dName}\n" +
                            "Issued by: ${cert?.issuedBy?.dName}"
                    )
                }

                val host = Uri.parse(error?.url ?: "").host ?: ""
                val isTrustedDomain = host.endsWith("obr57.ru") ||
                    host.endsWith("shkolove.ru") ||
                    host.endsWith("gosuslugi.ru") ||
                    host.endsWith("gu-st.ru")

                // Пропускаем ошибку недоверенного УЦ (SSL_UNTRUSTED) только для наших доменов
                // и только если сертификат валиден по срокам и имени хоста
                if (error != null &&
                    isTrustedDomain &&
                    !error.hasError(SslError.SSL_IDMISMATCH) &&
                    !error.hasError(SslError.SSL_EXPIRED) &&
                    !error.hasError(SslError.SSL_NOTYETVALID) &&
                    !error.hasError(SslError.SSL_DATE_INVALID)
                ) {
                    handler?.proceed()
                } else {
                    handler?.cancel()
                }
            }

            override fun shouldOverrideUrlLoading(
                view: WebView?,
                request: WebResourceRequest?
            ): Boolean {
                val url = request?.url?.toString() ?: return false
                if (isDebuggable) Log.d(TAG, "Redirect/Load: $url")

                if (url.contains("obr57.ru") || url.contains("shkolove.ru")) {
                    checkCookiesForToken(url)
                }
                return false
            }

            override fun onReceivedError(
                view: WebView?,
                request: WebResourceRequest?,
                error: WebResourceError?
            ) {
                super.onReceivedError(view, request, error)
                if (isDebuggable && request?.isForMainFrame == true) {
                    Log.e(
                        TAG,
                        "Load error: ${error?.description} (code ${error?.errorCode}) on ${request.url}"
                    )
                }
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

                    val storagePath = filesDir.absolutePath

                    sendTokenToRust(token, storagePath)
                    CookieManager.getInstance().removeAllCookies(null)
                    CookieManager.getInstance().flush()
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