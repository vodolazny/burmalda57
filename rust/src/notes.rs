// Локальные заметки к урокам. Хранятся зашифрованно на устройстве,
// по аналогии с homework.rs / events.rs.

use crate::crypto::{load_decrypted_file, save_encrypted_file};
use std::collections::HashMap;
use std::sync::Mutex;

const NOTES_FILE: &str = ".notes";
const CONTEXT_NOTES: &[u8] = b"burmalda57_notes_context_v1";

// Карта: "дата|номер|предмет" -> текст заметки
static NOTES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static STORAGE: Mutex<Option<String>> = Mutex::new(None);

// Собрать ключ заметки из (дата, номер урока, предмет).
pub(crate) fn key(date: &str, number: i32, subject: &str) -> String {
    format!("{}|{}|{}", date, number, subject.trim())
}

// Загрузить заметки с диска (вызывается один раз на старте, в фоновом потоке).
pub(crate) fn init(storage_path: &str) {
    *STORAGE.lock().unwrap() = Some(storage_path.to_string());
    let map = load_decrypted_file(storage_path, NOTES_FILE, CONTEXT_NOTES)
        .and_then(|json| serde_json::from_str::<HashMap<String, String>>(&json).ok())
        .unwrap_or_default();
    *NOTES.lock().unwrap() = Some(map);
}

// Сохранить текущее состояние в зашифрованный файл.
fn persist() {
    let storage = match STORAGE.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    let guard = NOTES.lock().unwrap();
    if let Some(map) = guard.as_ref() {
        match serde_json::to_string(map) {
            Ok(json) => {
                if let Err(e) = save_encrypted_file(&storage, NOTES_FILE, CONTEXT_NOTES, &json) {
                    log::warn!("Не удалось сохранить заметки: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать заметки: {:?}", e),
        }
    }
}

// Текст заметки по ключу (пустая строка, если заметки нет).
pub(crate) fn get(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    NOTES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(key).cloned())
        .unwrap_or_default()
}

// Сохранить/обновить заметку (autosave из UI). Пустой текст удаляет запись.
pub(crate) fn set(key: &str, text: &str) {
    if key.is_empty() {
        return;
    }
    let trimmed = text.trim();
    {
        let mut guard = NOTES.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        // Сохраняем только если текст реально изменился — autosave стреляет часто,
        // а шифрование через Keystore не бесплатное.
        let unchanged = match map.get(key) {
            Some(old) => old == trimmed,
            None => trimmed.is_empty(),
        };
        if unchanged {
            return;
        }
        if trimmed.is_empty() {
            map.remove(key);
        } else {
            map.insert(key.to_string(), trimmed.to_string());
        }
    }
    persist();
}

