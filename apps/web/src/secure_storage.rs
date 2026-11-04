// Шифрованное хранилище браузера: WebCrypto-конверт + localStorage.
//
// KEK — неизвлекаемый AES-256-GCM ключ из IndexedDB (wrapKey/unwrapKey),
// DEK — случайные 32 байта, завёрнутые KEK'ом в localStorage (`b57.wrapped-dek`).
// Сами данные шифрует core (AES-256-GCM + HKDF); адаптер только хранит blob'ы
// в localStorage под префиксом `b57.f.` и держит DEK в памяти wasm.
//
// Имена ключей и контексты — часть формата, менять нельзя: иначе у
// пользователей пропадут сохранённые сессии и кеши.

use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use burmalda57_core::crypto::NONCE_LEN;
use burmalda57_core::{decrypt_blob, encrypt_blob, random_bytes, SecureStore};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::window;

const LS_WRAPPED_DEK: &str = "b57.wrapped-dek";
const LS_FILE_PREFIX: &str = "b57.f.";
const IDB_NAME: &str = "burmalda57-keystore";
const IDB_STORE: &str = "keys";
const IDB_KEK: &str = "kek";

static DEK: Mutex<Option<[u8; 32]>> = Mutex::new(None);

pub(crate) struct WebStore;

impl SecureStore for WebStore {
    fn load(&self, _storage_path: &str, filename: &str, context: &[u8]) -> Option<String> {
        let dek = (*DEK.lock().unwrap())?;
        let b64 = ls().ok()?.get_item(&file_key(filename)).ok()??;
        let blob = STANDARD.decode(b64.as_bytes()).ok()?;
        decrypt_blob(&dek, context, &blob)
    }

    fn save(
        &self,
        _storage_path: &str,
        filename: &str,
        context: &[u8],
        data: &str,
    ) -> Result<(), String> {
        let dek = (*DEK.lock().unwrap()).ok_or("хранилище ключей не инициализировано")?;
        let blob = encrypt_blob(&dek, context, data)?;
        ls().map_err(|e| e.to_string())?
            .set_item(&file_key(filename), &STANDARD.encode(&blob))
            .map_err(|e| format!("{e:?}"))
    }

    fn delete(&self, _storage_path: &str, filename: &str) {
        if let Ok(storage) = ls() {
            let _ = storage.remove_item(&file_key(filename));
        }
    }
}

fn file_key(filename: &str) -> String {
    format!("{LS_FILE_PREFIX}{filename}")
}

// JsValue не реализует StdError — приводим к строке.
fn js_err(e: JsValue) -> Box<dyn std::error::Error> {
    format!("{e:?}").into()
}

fn subtle() -> Result<web_sys::SubtleCrypto, Box<dyn std::error::Error>> {
    Ok(window()
        .ok_or("нет window")?
        .crypto()
        .map_err(js_err)?
        .subtle())
}

fn ls() -> Result<web_sys::Storage, Box<dyn std::error::Error>> {
    window()
        .ok_or("нет window")?
        .local_storage()
        .map_err(js_err)?
        .ok_or("localStorage недоступен")
        .map_err(Into::into)
}

// Для generateKey у AES-GCM обязателен length; для wrapKey/unwrapKey —
// объект параметров с IV (голое имя алгоритма даёт AeadParams: iv missing).
fn aes_gcm_algo() -> Result<js_sys::Object, Box<dyn std::error::Error>> {
    Ok(js_sys::JSON::parse(r#"{"name":"AES-GCM","length":256}"#)
        .map_err(js_err)?
        .dyn_into()
        .map_err(js_err)?)
}

fn aead_params(iv: &[u8]) -> Result<js_sys::Object, Box<dyn std::error::Error>> {
    let params = js_sys::Object::new();
    js_sys::Reflect::set(
        &params,
        &JsValue::from_str("name"),
        &JsValue::from_str("AES-GCM"),
    )
    .map_err(js_err)?;
    js_sys::Reflect::set(
        &params,
        &JsValue::from_str("iv"),
        &js_sys::Uint8Array::from(iv),
    )
    .map_err(js_err)?;
    Ok(params)
}

fn usages(names: &[&str]) -> JsValue {
    let arr = js_sys::Array::new();
    for name in names {
        arr.push(&JsValue::from_str(name));
    }
    arr.into()
}

// =========================================================================
//  IndexedDB: неизвлекаемый KEK
// =========================================================================
fn idb_request(req: &web_sys::IdbRequest) -> JsFuture {
    let req = req.clone();
    JsFuture::from(js_sys::Promise::new(&mut move |resolve, reject| {
        let onsuccess = Closure::<dyn Fn()>::new({
            let resolve = resolve.clone();
            let req = req.clone();
            move || {
                // Резолвим результатом запроса: для open это IDBDatabase,
                // для get/put — значение/ключ. Сам запрос — не результат.
                let value = req.result().unwrap_or(JsValue::UNDEFINED);
                let _ = resolve.call1(&JsValue::NULL, &value);
            }
        });
        let onerror = Closure::<dyn Fn()>::new({
            let req = req.clone();
            move || {
                let err = req
                    .error()
                    .ok()
                    .flatten()
                    .map(JsValue::from)
                    .unwrap_or_else(|| JsValue::from_str("IndexedDB request failed"));
                let _ = reject.call1(&JsValue::NULL, &err);
            }
        });
        req.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
        req.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        // Одноразовые обработчики: сработают максимум один раз.
        onsuccess.forget();
        onerror.forget();
    }))
}

// Создаёт object store при первичном открытии базы. Обработчик обязателен
// уже для первого open: без него Chromium может прервать upgrade-транзакцию.
fn upgrade_handler() -> Closure<dyn Fn(web_sys::Event)> {
    Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
        if let Some(req) = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::IdbOpenDbRequest>().ok())
        {
            if let Ok(db) = req
                .result()
                .and_then(|r| r.dyn_into::<web_sys::IdbDatabase>())
            {
                let _ = db.create_object_store(IDB_STORE);
            }
        }
    })
}

async fn open_keystore_db() -> Result<web_sys::IdbDatabase, JsValue> {
    let win = window().ok_or_else(|| JsValue::from_str("нет window"))?;
    let factory = win
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB недоступна"))?;

    // Открываем базу без смены версии и проверяем наличие object store.
    let req = factory.open(IDB_NAME)?;
    let handler = upgrade_handler();
    req.set_onupgradeneeded(Some(handler.as_ref().unchecked_ref()));
    let opened = idb_request(&req).await?;
    drop(handler);
    let db: web_sys::IdbDatabase = opened.dyn_into()?;
    let has_store = db
        .transaction_with_str_and_mode(IDB_STORE, web_sys::IdbTransactionMode::Readonly)
        .is_ok();
    if has_store {
        return Ok(db);
    }
    let version = db.version() as u32;
    db.close();

    // Store нет — открываем с новой версией и создаём его в onupgradeneeded.
    let req = factory.open_with_u32(IDB_NAME, version + 1)?;
    let handler = upgrade_handler();
    req.set_onupgradeneeded(Some(handler.as_ref().unchecked_ref()));
    let opened = idb_request(&req).await?;
    drop(handler);
    let db: web_sys::IdbDatabase = opened.dyn_into()?;
    Ok(db)
}

async fn idb_get_kek() -> Result<Option<web_sys::CryptoKey>, JsValue> {
    let db = open_keystore_db().await?;
    let result = (async {
        let tx =
            db.transaction_with_str_and_mode(IDB_STORE, web_sys::IdbTransactionMode::Readonly)?;
        let store = tx.object_store(IDB_STORE)?;
        let req = store.get(&JsValue::from_str(IDB_KEK))?;
        let val = idb_request(&req).await?;
        if val.is_undefined() || val.is_null() {
            Ok(None)
        } else {
            Ok(Some(val.dyn_into::<web_sys::CryptoKey>()?))
        }
    })
    .await;
    db.close();
    result
}

async fn idb_put_kek(key: &web_sys::CryptoKey) -> Result<(), JsValue> {
    let db = open_keystore_db().await?;
    let result = (async {
        let tx =
            db.transaction_with_str_and_mode(IDB_STORE, web_sys::IdbTransactionMode::Readwrite)?;
        let store = tx.object_store(IDB_STORE)?;
        let req = store.put_with_key(&key.into(), &JsValue::from_str(IDB_KEK))?;
        idb_request(&req).await?;
        Ok(())
    })
    .await;
    db.close();
    result
}

// =========================================================================
//  Конверт DEK: base64(iv(12) || AES-GCM-шифртекст+тег)
// =========================================================================
async fn unwrap_dek(
    subtle: &web_sys::SubtleCrypto,
    kek: &web_sys::CryptoKey,
) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let stored_b64 = ls()?
        .get_item(LS_WRAPPED_DEK)
        .map_err(js_err)?
        .ok_or("завёрнутый DEK не найден")?;
    let stored = STANDARD.decode(stored_b64.as_bytes())?;
    if stored.len() <= NONCE_LEN {
        return Err("завёрнутый DEK повреждён".into());
    }
    let (iv, wrapped) = stored.split_at(NONCE_LEN);
    let params = aead_params(iv)?;

    let dek_val = JsFuture::from(
        subtle
            .unwrap_key_with_u8_array_and_object_and_str(
                "raw",
                wrapped,
                kek,
                &params,
                "AES-GCM",
                true, // extractable: только чтобы прочитать байты в память wasm
                &usages(&["encrypt"]),
            )
            .map_err(js_err)?,
    )
    .await
    .map_err(js_err)?;
    let dek_key: web_sys::CryptoKey = dek_val.dyn_into().map_err(js_err)?;

    let exported = JsFuture::from(subtle.export_key("raw", &dek_key).map_err(js_err)?)
        .await
        .map_err(js_err)?;
    let bytes = js_sys::Uint8Array::new(&exported).to_vec();
    if bytes.len() != 32 {
        return Err("DEK неверной длины".into());
    }
    let mut dek = [0u8; 32];
    dek.copy_from_slice(&bytes);
    Ok(dek)
}

async fn wrap_dek(
    subtle: &web_sys::SubtleCrypto,
    kek: &web_sys::CryptoKey,
    dek: &[u8; 32],
) -> Result<(), Box<dyn std::error::Error>> {
    let dek_key_val = JsFuture::from(
        subtle
            .import_key_with_str(
                "raw",
                &js_sys::Uint8Array::from(dek.as_slice()),
                "AES-GCM",
                true,
                &usages(&["encrypt"]),
            )
            .map_err(js_err)?,
    )
    .await
    .map_err(js_err)?;
    let dek_key: web_sys::CryptoKey = dek_key_val.dyn_into().map_err(js_err)?;

    let iv = random_bytes::<NONCE_LEN>();
    let params = aead_params(&iv)?;
    let wrapped_val = JsFuture::from(
        subtle
            .wrap_key_with_object("raw", &dek_key, kek, &params)
            .map_err(js_err)?,
    )
    .await
    .map_err(js_err)?;
    let wrapped = js_sys::Uint8Array::new(&wrapped_val).to_vec();

    let mut stored = Vec::with_capacity(NONCE_LEN + wrapped.len());
    stored.extend_from_slice(&iv);
    stored.extend_from_slice(&wrapped);
    ls()?
        .set_item(LS_WRAPPED_DEK, &STANDARD.encode(&stored))
        .map_err(js_err)?;
    Ok(())
}

fn has_encrypted_data() -> Result<bool, Box<dyn std::error::Error>> {
    let storage = ls()?;
    if storage.get_item(LS_WRAPPED_DEK).map_err(js_err)?.is_some() {
        return Ok(true);
    }
    for i in 0..storage.length().map_err(js_err)? {
        if storage
            .key(i)
            .map_err(js_err)?
            .is_some_and(|key| key.starts_with(LS_FILE_PREFIX))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

// =========================================================================
//  Инициализация при старте
// =========================================================================
/// Serialize initialization across tabs, including asynchronous WebCrypto calls.
pub(crate) async fn init_keystore() -> Result<(), Box<dyn std::error::Error>> {
    let win = window().ok_or("нет window")?;
    let navigator =
        js_sys::Reflect::get(win.as_ref(), &JsValue::from_str("navigator")).map_err(js_err)?;
    let locks = js_sys::Reflect::get(&navigator, &JsValue::from_str("locks")).map_err(js_err)?;
    if locks.is_null() || locks.is_undefined() {
        return Err("Для защищённого хранилища требуется браузер с Web Locks и HTTPS".into());
    }
    let request: js_sys::Function = js_sys::Reflect::get(&locks, &JsValue::from_str("request"))
        .map_err(js_err)?
        .dyn_into()
        .map_err(js_err)?;
    let callback = Closure::<dyn FnMut() -> js_sys::Promise>::once(|| {
        wasm_bindgen_futures::future_to_promise(async {
            init_keystore_locked()
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            Ok(JsValue::UNDEFINED)
        })
    });
    let promise: js_sys::Promise = request
        .call2(
            &locks,
            &JsValue::from_str("burmalda57-keystore"),
            callback.as_ref(),
        )
        .map_err(js_err)?
        .dyn_into()
        .map_err(js_err)?;
    JsFuture::from(promise).await.map_err(js_err)?;
    drop(callback);
    Ok(())
}

async fn init_keystore_locked() -> Result<(), Box<dyn std::error::Error>> {
    let subtle = subtle()?;

    if let Some(kek) = idb_get_kek().await.map_err(js_err)? {
        match unwrap_dek(&subtle, &kek).await {
            Ok(dek) => {
                *DEK.lock().unwrap() = Some(dek);
                log::info!("Хранилище сессии разблокировано (DEK восстановлен)");
                return Ok(());
            }
            Err(e) => {
                // Recover an interrupted FIRST initialization only. Never replace
                // an existing envelope or a key protecting encrypted user files.
                if has_encrypted_data()? {
                    return Err(e);
                }
                let dek = random_bytes::<32>();
                wrap_dek(&subtle, &kek, &dek).await?;
                *DEK.lock().unwrap() = Some(dek);
                return Ok(());
            }
        }
    }

    if has_encrypted_data()? {
        return Err(
            "Ключ IndexedDB недоступен; существующие данные сохранены без изменений".into(),
        );
    }
    // Первый запуск: KEK (неизвлекаемый) + DEK.
    let algo = aes_gcm_algo()?;
    let kek_val = JsFuture::from(
        subtle
            .generate_key_with_object(&algo, false, &usages(&["wrapKey", "unwrapKey"]))
            .map_err(js_err)?,
    )
    .await
    .map_err(js_err)?;
    let kek: web_sys::CryptoKey = kek_val.dyn_into().map_err(js_err)?;
    idb_put_kek(&kek).await.map_err(js_err)?;

    let dek = random_bytes::<32>();
    wrap_dek(&subtle, &kek, &dek).await?;
    *DEK.lock().unwrap() = Some(dek);
    log::info!("Создано новое хранилище сессии (KEK + DEK)");
    Ok(())
}
