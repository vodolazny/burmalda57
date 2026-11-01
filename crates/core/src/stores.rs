// Локальные пользовательские данные: события, отметки ДЗ, заметки и фильтр
// предметов. Форматы JSON совпадают с прежними версиями приложений.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::crypto::{
    CONTEXT_EVENTS, CONTEXT_HOMEWORK, CONTEXT_NOTES, CONTEXT_SUBJECTS, FILE_EVENTS, FILE_HOMEWORK,
    FILE_NOTES, FILE_SUBJECTS,
};
use crate::model::{StoredEvent, SubjectToggleView};
use crate::ports::SecureStore;

// ============================================================
//  Свои события
// ============================================================
pub struct EventsStore {
    map: HashMap<String, Vec<StoredEvent>>,
}

impl EventsStore {
    pub fn load(store: &dyn SecureStore, path: &str) -> Self {
        let map = store
            .load(path, FILE_EVENTS, CONTEXT_EVENTS)
            .and_then(|json| serde_json::from_str::<HashMap<String, Vec<StoredEvent>>>(&json).ok())
            .unwrap_or_default();
        Self { map }
    }

    pub fn for_date(&self, date: &str) -> Vec<StoredEvent> {
        self.map.get(date).cloned().unwrap_or_default()
    }

    pub fn add(
        &mut self,
        store: &dyn SecureStore,
        path: &str,
        date: &str,
        name: &str,
        start: &str,
        end: &str,
    ) {
        // Event IDs are opaque. Browser entropy also works on wasm, where
        // std::time::SystemTime::now() is unsupported and panics.
        let id = format!("{:032x}", rand::random::<u128>());
        self.map.entry(date.to_string()).or_default().push(StoredEvent {
            id,
            name: name.trim().to_string(),
            start: normalize_time(start),
            end: normalize_time(end),
        });
        self.persist(store, path);
    }

    pub fn delete(&mut self, store: &dyn SecureStore, path: &str, date: &str, id: &str) {
        if let Some(list) = self.map.get_mut(date) {
            list.retain(|e| e.id != id);
            if list.is_empty() {
                self.map.remove(date);
            }
        }
        self.persist(store, path);
    }

    fn persist(&self, store: &dyn SecureStore, path: &str) {
        match serde_json::to_string(&self.map) {
            Ok(json) => {
                if let Err(e) = store.save(path, FILE_EVENTS, CONTEXT_EVENTS, &json) {
                    log::warn!("Не удалось сохранить события: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать события: {:?}", e),
        }
    }
}

// Формат "H:MM"/"HH:MM"; пустая строка корректна (поле «до» необязательно).
pub fn is_valid_time(s: &str) -> bool {
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
    let (Ok(hh), Ok(mm)) = (h.parse::<u32>(), m.parse::<u32>()) else {
        return false;
    };
    hh <= 23 && mm <= 59
}

fn normalize_time(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let mut it = s.splitn(2, |c| c == ':' || c == '.' || c == ' ');
    let digits = |x: &str| -> u32 {
        let d: String = x.chars().filter(|c| c.is_ascii_digit()).collect();
        d.parse().unwrap_or(0)
    };
    let mut h = digits(it.next().unwrap_or(""));
    let mut m = digits(it.next().unwrap_or(""));
    if h > 23 {
        h = 23;
    }
    if m > 59 {
        m = 59;
    }
    format!("{:02}:{:02}", h, m)
}

// ============================================================
//  Домашка
// ============================================================
pub struct HomeworkStore {
    done: HashSet<String>,
}

impl HomeworkStore {
    pub fn load(store: &dyn SecureStore, path: &str) -> Self {
        let done = store
            .load(path, FILE_HOMEWORK, CONTEXT_HOMEWORK)
            .and_then(|json| serde_json::from_str::<HashSet<String>>(&json).ok())
            .unwrap_or_default();
        Self { done }
    }

    pub fn is_done(&self, key: &str) -> bool {
        !key.is_empty() && self.done.contains(key)
    }

    pub fn set_done(&mut self, store: &dyn SecureStore, path: &str, key: &str, done: bool) {
        if key.is_empty() {
            return;
        }
        if done {
            self.done.insert(key.to_string());
        } else {
            self.done.remove(key);
        }
        self.persist(store, path);
    }

    fn persist(&self, store: &dyn SecureStore, path: &str) {
        match serde_json::to_string(&self.done) {
            Ok(json) => {
                if let Err(e) = store.save(path, FILE_HOMEWORK, CONTEXT_HOMEWORK, &json) {
                    log::warn!("Не удалось сохранить отметки ДЗ: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать отметки ДЗ: {:?}", e),
        }
    }
}

// ============================================================
//  Заметки к урокам
// ============================================================
pub struct NotesStore {
    map: HashMap<String, String>,
}

impl NotesStore {
    pub fn load(store: &dyn SecureStore, path: &str) -> Self {
        let map = store
            .load(path, FILE_NOTES, CONTEXT_NOTES)
            .and_then(|json| serde_json::from_str::<HashMap<String, String>>(&json).ok())
            .unwrap_or_default();
        Self { map }
    }

    /// Ключ заметки: (дата, номер урока, предмет). Серверный id не подходит:
    /// он меняется при правках расписания.
    pub fn key(date: &str, number: i32, subject: &str) -> String {
        format!("{}|{}|{}", date, number, subject.trim())
    }

    pub fn get(&self, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        self.map.get(key).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, store: &dyn SecureStore, path: &str, key: &str, text: &str) {
        if key.is_empty() {
            return;
        }
        let trimmed = text.trim();
        let unchanged = match self.map.get(key) {
            Some(old) => old == trimmed,
            None => trimmed.is_empty(),
        };
        if unchanged {
            return;
        }
        if trimmed.is_empty() {
            self.map.remove(key);
        } else {
            self.map.insert(key.to_string(), trimmed.to_string());
        }
        self.persist(store, path);
    }

    fn persist(&self, store: &dyn SecureStore, path: &str) {
        match serde_json::to_string(&self.map) {
            Ok(json) => {
                if let Err(e) = store.save(path, FILE_NOTES, CONTEXT_NOTES, &json) {
                    log::warn!("Не удалось сохранить заметки: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать заметки: {:?}", e),
        }
    }
}

// ============================================================
//  Фильтр предметов
// ============================================================
#[derive(Serialize, Deserialize, Default)]
struct SubjectsStored {
    #[serde(default)]
    known: BTreeSet<String>,
    #[serde(default)]
    hidden: BTreeSet<String>,
}

pub struct SubjectsStore {
    state: SubjectsStored,
}

impl SubjectsStore {
    pub fn load(store: &dyn SecureStore, path: &str) -> Self {
        let state = store
            .load(path, FILE_SUBJECTS, CONTEXT_SUBJECTS)
            .and_then(|json| serde_json::from_str::<SubjectsStored>(&json).ok())
            .unwrap_or_default();
        Self { state }
    }

    pub fn is_hidden(&self, subject: &str) -> bool {
        let key = subject.trim();
        !key.is_empty() && self.state.hidden.contains(key)
    }

    /// Запомнить встретившиеся предметы. true — список изменился.
    pub fn note_subjects<I, S>(&mut self, store: &dyn SecureStore, path: &str, subjects: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut changed = false;
        for s in subjects {
            let name = s.as_ref().trim();
            if !name.is_empty() && self.state.known.insert(name.to_string()) {
                changed = true;
            }
        }
        if changed {
            self.persist(store, path);
        }
        changed
    }

    pub fn set_hidden(
        &mut self,
        store: &dyn SecureStore,
        path: &str,
        subject: &str,
        hidden: bool,
    ) {
        let key = subject.trim().to_string();
        if key.is_empty() {
            return;
        }
        self.state.known.insert(key.clone());
        if hidden {
            self.state.hidden.insert(key);
        } else {
            self.state.hidden.remove(&key);
        }
        self.persist(store, path);
    }

    pub fn reset(&mut self, store: &dyn SecureStore, path: &str) {
        self.state = SubjectsStored::default();
        self.persist(store, path);
    }

    pub fn toggles(&self) -> Vec<SubjectToggleView> {
        self.state
            .known
            .iter()
            .map(|s| SubjectToggleView {
                subject: s.clone(),
                hidden: self.state.hidden.contains(s),
            })
            .collect()
    }

    fn persist(&self, store: &dyn SecureStore, path: &str) {
        match serde_json::to_string(&self.state) {
            Ok(json) => {
                if let Err(e) = store.save(path, FILE_SUBJECTS, CONTEXT_SUBJECTS, &json) {
                    log::warn!("Не удалось сохранить фильтр предметов: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать фильтр предметов: {:?}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore {
        data: Mutex<HashMap<String, String>>,
    }

    impl SecureStore for MemStore {
        fn load(&self, _p: &str, f: &str, _c: &[u8]) -> Option<String> {
            self.data.lock().unwrap().get(f).cloned()
        }
        fn save(&self, _p: &str, f: &str, _c: &[u8], d: &str) -> Result<(), String> {
            self.data.lock().unwrap().insert(f.to_string(), d.to_string());
            Ok(())
        }
        fn delete(&self, _p: &str, f: &str) {
            self.data.lock().unwrap().remove(f);
        }
    }

    #[test]
    fn validates_time() {
        assert!(is_valid_time(""));
        assert!(is_valid_time("8:05"));
        assert!(is_valid_time("23:59"));
        assert!(!is_valid_time("24:00"));
        assert!(!is_valid_time("12:60"));
        assert!(!is_valid_time("12"));
        assert!(!is_valid_time("1:2:3"));
    }

    #[test]
    fn normalizes_event_time() {
        let store = MemStore::default();
        let mut events = EventsStore::load(&store, "p");
        events.add(&store, "p", "2026-01-01", " Тренировка ", "8.5", "25:99");
        let e = &events.for_date("2026-01-01")[0];
        assert_eq!(e.name, "Тренировка");
        assert_eq!(e.start, "08:05");
        assert_eq!(e.end, "23:59");
    }

    #[test]
    fn event_ids_remain_distinct_after_reload_and_old_ids_can_be_deleted() {
        let store = MemStore::default();
        // Preserve compatibility with IDs saved by earlier application versions.
        store.save("p", FILE_EVENTS, CONTEXT_EVENTS,
            r#"{"2026-01-01":[{"id":"1700000000000-1","name":"Старое событие","start":"08:00","end":""}]}"#).unwrap();
        let mut events = EventsStore::load(&store, "p");
        events.add(&store, "p", "2026-01-01", "Первое", "09:00", "");
        let mut events = EventsStore::load(&store, "p");
        events.add(&store, "p", "2026-01-01", "Второе", "09:00", "");
        let items = events.for_date("2026-01-01");
        let ids: HashSet<_> = items.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), 3);
        events.delete(&store, "p", "2026-01-01", "1700000000000-1");
        let remaining = EventsStore::load(&store, "p").for_date("2026-01-01");
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().all(|e| e.id != "1700000000000-1"));
    }

    #[test]
    fn notes_ignore_unchanged_and_remove_empty() {
        let store = MemStore::default();
        let mut notes = NotesStore::load(&store, "p");
        let key = NotesStore::key("2026-01-01", 1, "Алгебра");
        notes.set(&store, "p", &key, "  текст ");
        assert_eq!(notes.get(&key), "текст");
        notes.set(&store, "p", &key, "текст");
        notes.set(&store, "p", &key, "   ");
        assert_eq!(notes.get(&key), "");
    }
}
