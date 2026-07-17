// crypto.rs — хранение сессии и кэша с защитой ключа через Android Keystore.
//
// Схема (envelope encryption):
//   * DEK  — случайные 32 байта, ими AES-256-GCM шифруются сами файлы.
//   * KEK  — ключ AES/GCM в AndroidKeyStore (TEE/StrongBox), не покидает железо.
//            Им "заворачивается" DEK; на диске лежит только обёртка (.dek).
//   * HKDF-SHA256 разделяет ключи по контексту (.session / .marks_cache).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use aes::cipher::{BlockEncryptMut, KeyInit as AesKeyInit};
use aes::Aes128;
use base64::{engine::general_purpose::STANDARD, Engine as _};

pub const CONTEXT_SESSION: &[u8] = b"burmalda57_session_context_v1";
pub const CONTEXT_MARKS_CACHE: &[u8] = b"burmalda57_marks_cache_context_v1";

const DEK_FILE: &str = ".dek";
const NONCE_LEN: usize = 12;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserSession {
    pub sid: String,          // кука X1_SSO (нужна для дальнейших запросов)
    pub user_guid: String,    // PARTICIPANT.SYS_GUID — твой гуид
    pub apikey: String,       // Сессионный apikey
    pub full_name: String,    // ФИО: SURNAME NAME SECONDNAME
    pub school_name: String,  // SCHOOL.NAME — школа
    pub school_class: String, // GRADE.NAME — класс (напр. "8Г")
}

// =========================================================================
//  Реверс-инжиниринговый метод для генерации apikey 
// =========================================================================
pub fn ahh_encrypt(text: &str) -> String {
    let key_bytes: [u8; 16] = [31, 23, 19, 50, 40, 23, 19, 10, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut encrypted_bytes = text.as_bytes().to_vec();
    
    let block_size = 16;
    let padding_len = block_size - (encrypted_bytes.len() % block_size);
    encrypted_bytes.resize(encrypted_bytes.len() + padding_len, padding_len as u8);

    // Шифрование AES-128-ECB
    let mut cipher = Aes128::new(&key_bytes.into());
    for chunk in encrypted_bytes.chunks_exact_mut(16) {
        let block = aes::cipher::generic_array::GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block_mut(block);
    }
    
    // Кодирование в Base64 с флагом NO_WRAP (убираем переносы строк)
    let encoded = STANDARD.encode(encrypted_bytes);
    encoded.replace("\n", "").replace("\r", "")
}

// =========================================================================
//  Права доступа к файлам (0600 на unix/android)
// =========================================================================
#[cfg(unix)]
fn restrict_permissions(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &str) {}

// ============================================================
//  KEK: заворачивание/разворачивание DEK через Android Keystore
// ============================================================
#[cfg(target_os = "android")]
mod keystore {
    use jni::objects::{JByteArray, JValue, JObject, JClass};
    use jni::JavaVM;

    fn with_env<R>(
        f: impl FnOnce(&mut jni::JNIEnv) -> jni::errors::Result<R>,
    ) -> Result<R, Box<dyn std::error::Error>> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        Ok(f(&mut env)?)
    }

    fn call(method: &str, input: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        with_env(|env| {
            let ctx = ndk_context::android_context();
            let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

            // Очищаем старые эксепшены, если они были
            if let Ok(true) = env.exception_check() {
                let _ = env.exception_clear();
            }

            // 1. Достаём ClassLoader нашего Activity
            let class_loader = match env.call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]) {
                Ok(res) => res.l()?,
                Err(e) => {
                    if let Ok(true) = env.exception_check() { let _ = env.exception_clear(); }
                    return Err(e);
                }
            };

            // 2. Загружаем класс KeystoreCrypto через ClassLoader приложения
            let class_name = env.new_string("com.burmalda57.crypto.KeystoreCrypto")?;
            let class_obj = match env.call_method(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_name)],
            ) {
                Ok(res) => res.l()?,
                Err(e) => {
                    if let Ok(true) = env.exception_check() { 
                        let _ = env.exception_describe(); // Выведет трейс в логкат, если что-то пойдёт не так
                        let _ = env.exception_clear(); 
                    }
                    return Err(e);
                }
            };

            let clazz = JClass::from(class_obj);
            let arg = env.byte_array_from_slice(input)?;
            
            // 3. Дёргаем статический метод
            let res: JByteArray = match env.call_static_method(
                &clazz,
                method,
                "([B)[B",
                &[JValue::Object(&arg)],
            ) {
                Ok(res) => res.l()?.into(),
                Err(e) => {
                    if let Ok(true) = env.exception_check() { let _ = env.exception_clear(); }
                    return Err(e);
                }
            };

            env.convert_byte_array(res)
        })
        .map_err(Into::into)
    }

    pub fn wrap(plain: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        call("wrap", plain)
    }
    pub fn unwrap(blob: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        call("unwrap", blob)
    }
}

#[cfg(target_os = "android")]
fn kek_wrap(plain: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    keystore::wrap(plain)
}
#[cfg(target_os = "android")]
fn kek_unwrap(blob: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    keystore::unwrap(blob)
}

// =========================================================================
//  DEK: получить или создать (защищён KEK'ом Keystore)
// =========================================================================
fn get_or_create_dek(storage_path: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let dek_path = format!("{}/{}", storage_path, DEK_FILE);

    if Path::new(&dek_path).exists() {
        let mut blob = Vec::new();
        File::open(&dek_path)?.read_to_end(&mut blob)?;
        if !blob.is_empty() {
            if let Ok(dek) = kek_unwrap(&blob) {
                if dek.len() == 32 {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&dek);
                    return Ok(out);
                }
            }
        }
    }

    // Новый DEK
    let mut dek = [0u8; 32];
    thread_rng().fill_bytes(&mut dek);

    let blob = kek_wrap(&dek)?;
    let mut file = File::create(&dek_path)?;
    file.write_all(&blob)?;
    drop(file);
    restrict_permissions(&dek_path);

    Ok(dek)
}

// =========================================================================
//  Вывод рабочего ключа: IKM = секретный DEK, info = контекст.
// =========================================================================
fn derive_key_from_dek(
    dek: &[u8; 32],
    context_info: &[u8],
) -> Result<Key<Aes256Gcm>, Box<dyn std::error::Error>> {
    let hk = Hkdf::<Sha256>::new(None, dek);
    let mut okm = [0u8; 32];
    hk.expand(context_info, &mut okm)
        .map_err(|_| "Ошибка расширения ключа в HKDF")?;
    Ok(*Key::<Aes256Gcm>::from_slice(&okm))
}

// =========================================================================
//  Сохранение / загрузка зашифрованных файлов (БЕЗ ВЕРСИЙ)
// =========================================================================
// Формат файла: [12 байт nonce][ciphertext+tag]
pub fn save_encrypted_file(
    storage_path: &str,
    filename: &str,
    context_info: &[u8],
    data: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let dek = get_or_create_dek(storage_path)?;
    let key = derive_key_from_dek(&dek, context_info)?;
    let cipher = Aes256Gcm::new(&key);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let encrypted_data = cipher
        .encrypt(nonce, data.as_bytes())
        .map_err(|e| format!("Ошибка шифрования: {:?}", e))?;

    let file_path = format!("{}/{}", storage_path, filename);
    let mut file = File::create(&file_path)?;
    
    // Пишем строго nonce и криптотекст
    file.write_all(&nonce_bytes)?;
    file.write_all(&encrypted_data)?;
    drop(file);

    restrict_permissions(&file_path);
    Ok(())
}

pub fn load_decrypted_file(
    storage_path: &str,
    filename: &str,
    context_info: &[u8],
) -> Option<String> {
    let mut file = File::open(format!("{}/{}", storage_path, filename)).ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;

    // минимум: nonce(12) + хотя бы 1 байт данных + tag(16)
    if buffer.len() < NONCE_LEN + 1 + 16 {
        return None;
    }

    // Читаем с самого начала файла, без всяких сдвигов!
    let nonce_bytes = &buffer[0..NONCE_LEN];
    let encrypted_data = &buffer[NONCE_LEN..];

    let dek = get_or_create_dek(storage_path).ok()?;
    let key = derive_key_from_dek(&dek, context_info).ok()?;
    let cipher = Aes256Gcm::new(&key);
    let nonce = Nonce::from_slice(nonce_bytes);

    let decrypted_bytes = cipher.decrypt(nonce, encrypted_data).ok()?;
    String::from_utf8(decrypted_bytes).ok()
}

// =========================================================================
//  Высокоуровневый API
// =========================================================================
pub fn save_session(
    storage_path: &str,
    session: &UserSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let json_str = serde_json::to_string(session)?;
    save_encrypted_file(storage_path, ".session", CONTEXT_SESSION, &json_str)
}

pub fn load_session(storage_path: &str) -> Option<UserSession> {
    let json_str = load_decrypted_file(storage_path, ".session", CONTEXT_SESSION)?;
    serde_json::from_str(&json_str).ok()
}

pub fn save_marks_cache(
    storage_path: &str,
    marks_json: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    save_encrypted_file(storage_path, ".marks_cache", CONTEXT_MARKS_CACHE, marks_json)
}

pub fn load_marks_cache(storage_path: &str) -> Option<String> {
    load_decrypted_file(storage_path, ".marks_cache", CONTEXT_MARKS_CACHE)
}
