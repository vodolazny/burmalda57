// Локальные отметки «домашка выполнена». Хранятся зашифрованно на устройстве,
// по аналогии с events.rs. Ключ = "дата|номер|предмет".
use crate::crypto::{load_decrypted_file, save_encrypted_file};
use std::collections::HashSet;
use std::sync::Mutex;

const HOMEWORK_FILE: &str = ".homework";
const CONTEXT_HOMEWORK: &[u8] = b"burmalda57_homework_context_v1";

// Множество ключей выполненных ДЗ
static DONE: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static STORAGE: Mutex<Option<String>> = Mutex::new(None);

// Загрузить отметки с диска (вызывается один раз при старте, как events::init).
pub(crate) fn init(storage_path: &str) {
    *STORAGE.lock().unwrap() = Some(storage_path.to_string());
    let set = load_decrypted_file(storage_path, HOMEWORK_FILE, CONTEXT_HOMEWORK)
        .and_then(|json| serde_json::from_str::<HashSet<String>>(&json).ok())
        .unwrap_or_default();
    *DONE.lock().unwrap() = Some(set);
}

// Сохранить текущее множество в зашифрованный файл.
fn persist() {
    let storage = match STORAGE.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    let guard = DONE.lock().unwrap();
    if let Some(set) = guard.as_ref() {
        match serde_json::to_string(set) {
            Ok(json) => {
                if let Err(e) =
                    save_encrypted_file(&storage, HOMEWORK_FILE, CONTEXT_HOMEWORK, &json)
                {
                    log::warn!("Не удалось сохранить отметки ДЗ: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать отметки ДЗ: {:?}", e),
        }
    }
}

// Выполнена ли домашка с таким ключом.
pub(crate) fn is_done(key: &str) -> bool {
    DONE.lock()
        .unwrap()
        .as_ref()
        .map(|s| s.contains(key))
        .unwrap_or(false)
}

// Отметить/снять отметку выполнения и сразу сохранить.
pub(crate) fn set_done(key: &str, done: bool) {
    {
        let mut guard = DONE.lock().unwrap();
        let set = guard.get_or_insert_with(HashSet::new);
        if done {
            set.insert(key.to_string());
        } else {
            set.remove(key);
        }
    }
    persist();
}
