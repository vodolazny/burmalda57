package ru.burmalda.journal

import java.util.Locale

internal object AuthDomains {
    private fun belongsTo(host: String?, domains: List<String>): Boolean {
        val name = host?.lowercase(Locale.ROOT)?.trimEnd('.') ?: return false
        return domains.any { name == it || name.endsWith(".$it") }
    }

    fun isTrustedAuthHost(host: String?): Boolean =
        belongsTo(host, listOf("obr57.ru", "shkolove.ru", "gosuslugi.ru", "gu-st.ru"))

    fun isJournalHost(host: String?): Boolean =
        belongsTo(host, listOf("obr57.ru", "shkolove.ru"))
}
