// Свои события пользователя (не из журнала): тренировки, кружки и т.п.
// Хранятся зашифрованно на устройстве, отдельно от кеша дней журнала.

use crate::crypto::{load_decrypted_file, save_encrypted_file};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const EVENTS_FILE: &str = ".events";
const CONTEXT_EVENTS: &[u8] = b"burmalda57_events_context_v1";

#[derive(Serialize, Deserialize, Clone)]
pub struct StoredEvent {
    pub id: String,
    pub name: String,
    pub start: String,
    pub end: String,
}

// Карта: дата ("YYYY-MM-DD") -> список событий этого дня.
static EVENTS: Mutex<Option<HashMap<String, Vec<StoredEvent>>>> = Mutex::new(None);
static STORAGE: Mutex<Option<String>> = Mutex::new(None);

// Загрузить события с диска. Вызывать один раз при старте (в фоновом потоке,
// т.к. дешифрование обращается к Android Keystore).
pub(crate) fn init(storage_path: &str) {
    *STORAGE.lock().unwrap() = Some(storage_path.to_string());
    let map = load_decrypted_file(storage_path, EVENTS_FILE, CONTEXT_EVENTS)
        .and_then(|json| serde_json::from_str::<HashMap<String, Vec<StoredEvent>>>(&json).ok())
        .unwrap_or_default();
    *EVENTS.lock().unwrap() = Some(map);
}

// Сохранить текущее состояние на диск (зашифрованно).
fn persist() {
    let storage = match STORAGE.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    let guard = EVENTS.lock().unwrap();
    if let Some(map) = guard.as_ref() {
        match serde_json::to_string(map) {
            Ok(json) => {
                if let Err(e) = save_encrypted_file(&storage, EVENTS_FILE, CONTEXT_EVENTS, &json) {
                    log::warn!("Не удалось сохранить события: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать события: {:?}", e),
        }
    }
}

// События выбранного дня.
pub(crate) fn for_date(date: &str) -> Vec<StoredEvent> {
    EVENTS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(date))
        .cloned()
        .unwrap_or_default()
}

// Корректно ли время: формат "H:MM"/"HH:MM", только цифры и одно двоеточие,
// часы 0–23, минуты 0–59. Пустая строка — корректна (поле «до» необязательно).
pub(crate) fn is_valid_time(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    let mut parts = s.split(':');
    let h = match parts.next() {
        Some(x) => x,
        None => return false,
    };
    let m = match parts.next() {
        Some(x) => x,
        None => return false,
    };
    if parts.next().is_some() {
        return false;
    }
    if h.is_empty() || m.is_empty() || h.len() > 2 || m.len() > 2 {
        return false;
    }
    if !h.chars().all(|c| c.is_ascii_digit()) || !m.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let hh: u32 = match h.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mm: u32 = match m.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    hh <= 23 && mm <= 59
}

// Привести время к виду "HH:MM" и ограничить диапазоном 00:00–23:59.
// Пустая строка остаётся пустой (событие без времени окончания).
fn normalize_time(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let mut it = s.splitn(2, |c| c == ':' || c == '.' || c == ' ');
    let h_raw = it.next().unwrap_or("");
    let m_raw = it.next().unwrap_or("");
    let digits = |x: &str| -> u32 {
        let d: String = x.chars().filter(|c| c.is_ascii_digit()).collect();
        d.parse().unwrap_or(0)
    };
    let mut h = digits(h_raw);
    let mut m = digits(m_raw);
    if h > 23 {
        h = 23;
    }
    if m > 59 {
        m = 59;
    }
    format!("{:02}:{:02}", h, m)
}

// Добавить событие в указанный день и сохранить.
pub(crate) fn add(date: &str, name: &str, start: &str, end: &str) {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string();
    {
        let mut guard = EVENTS.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        map.entry(date.to_string()).or_default().push(StoredEvent {
            id,
            name: name.trim().to_string(),
            start: normalize_time(start),
            end: normalize_time(end),
        });
    }
    persist();
}

// Удалить событие по id из указанного дня и сохранить.
pub(crate) fn delete(date: &str, id: &str) {
    {
        let mut guard = EVENTS.lock().unwrap();
        if let Some(map) = guard.as_mut() {
            if let Some(list) = map.get_mut(date) {
                list.retain(|e| e.id != id);
                if list.is_empty() {
                    map.remove(date);
                }
            }
        }
    }
    persist();
}
