-dontobfuscate
-dontoptimize

-keep class ru.burmalda.journal.Notifier { *; }
-keep class ru.burmalda.journal.AvatarPickerActivity { *; }
-keep class ru.burmalda.journal.EsiaAuthActivity { *; }
-keep class com.burmalda57.crypto.KeystoreCrypto { *; }
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}
-keep class android.app.NativeActivity { *; }