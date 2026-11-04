package ru.burmalda.journal

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AuthDomainsTest {
    @Test fun acceptsOnlyTheDomainOrItsSubdomains() {
        for (domain in listOf("obr57.ru", "shkolove.ru", "gosuslugi.ru", "gu-st.ru")) {
            assertTrue(AuthDomains.isTrustedAuthHost(domain))
            assertTrue(AuthDomains.isTrustedAuthHost("login.$domain"))
            assertFalse(AuthDomains.isTrustedAuthHost("evil$domain"))
            assertFalse(AuthDomains.isTrustedAuthHost("$domain.evil.example"))
        }
        assertTrue(AuthDomains.isTrustedAuthHost("LOGIN.GOSUSLUGI.RU."))
        assertFalse(AuthDomains.isTrustedAuthHost(null))
        assertFalse(AuthDomains.isTrustedAuthHost(""))
        assertFalse(AuthDomains.isJournalHost("gosuslugi.ru"))
    }
}
