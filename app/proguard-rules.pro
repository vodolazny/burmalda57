-dontobfuscate
-dontoptimize

-keep class ru.burmalda.journal.Notifier { *; }
-keep class ru.burmalda.journal.GradesWorker { *; }
-keep class * extends androidx.work.ListenableWorker { <init>(...); }
-keep class ru.burmalda.journal.AvatarPickerActivity { *; }
-keep class ru.burmalda.journal.EsiaAuthActivity { *; }
-keep class ru.burmalda.journal.KeystoreCrypto { *; }
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}
-keep class android.app.NativeActivity { *; }