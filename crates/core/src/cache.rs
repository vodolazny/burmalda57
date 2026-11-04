// Кеш дней дневника: память текущей сессии + зашифрованный снимок на диске.
//
// Формат на диске:
//   * мобильные — {"days": {...}, "stamps": {...}};
//   * веб исторически — просто {...: дата -> сырой ответ}.
// Читаем оба, пишем в формате платформы (CacheFormat), чтобы старые версии
// веба продолжали понимать свой кеш.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::crypto::{CONTEXT_MARKS_CACHE, FILE_MARKS_CACHE};
use crate::ports::{CacheFormat, SecureStore};

pub const MAX_DAYS: usize = 120;

#[derive(Serialize, Deserialize, Default)]
struct DiskCache {
    #[serde(default)]
    days: HashMap<String, String>,
    #[serde(default)]
    stamps: HashMap<String, i64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DiskCacheCompat {
    // Старый формат первым: у него значения-строки, новый формат
    // (значения-объекты) в него не разберётся.
    Legacy(HashMap<String, String>),
    Current(DiskCache),
}

#[derive(Default)]
pub struct DayCache {
    days: HashMap<String, String>,
    stamps: HashMap<String, i64>,
    fetched: HashSet<String>,
}

impl DayCache {
    pub fn load(store: &dyn SecureStore, storage_path: &str) -> Self {
        let Some(json) = store.load(storage_path, FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE) else {
            return Self::default();
        };
        match serde_json::from_str::<DiskCacheCompat>(&json) {
            Ok(DiskCacheCompat::Legacy(map)) => {
                log::info!(
                    "Кеш дней загружен с диска (старый формат): {} дней",
                    map.len()
                );
                Self {
                    days: map,
                    ..Default::default()
                }
            }
            Ok(DiskCacheCompat::Current(c)) => {
                log::info!("Кеш дней загружен с диска: {} дней", c.days.len());
                Self {
                    days: c.days,
                    stamps: c.stamps,
                    fetched: HashSet::new(),
                }
            }
            Err(e) => {
                log::warn!("Не удалось разобрать кеш дней: {:?}", e);
                Self::default()
            }
        }
    }

    pub fn get_raw(&self, date: &str) -> Option<String> {
        self.days.get(date).cloned()
    }

    pub fn fetched_at(&self, date: &str) -> Option<i64> {
        self.stamps.get(date).copied()
    }

    pub fn is_fetched(&self, date: &str) -> bool {
        self.fetched.contains(date)
    }

    pub fn mark_fetched(&mut self, date: &str) {
        self.fetched.insert(date.to_string());
    }

    /// Свежий ответ из сети: в память + отметка времени + «загружен в сессии».
    pub fn put_mem(&mut self, date: &str, raw: &str) {
        self.days.insert(date.to_string(), raw.to_string());
        self.stamps
            .insert(date.to_string(), chrono::Local::now().timestamp());
        self.mark_fetched(date);
    }

    /// Положить в память, не помечая день как загруженный в этой сессии.
    pub fn put_mem_no_mark(&mut self, date: &str, raw: &str) {
        self.days.insert(date.to_string(), raw.to_string());
        self.stamps
            .insert(date.to_string(), chrono::Local::now().timestamp());
    }

    pub fn invalidate(&mut self, date: &str) {
        self.fetched.remove(date);
    }

    pub fn clear_session_marks(&mut self) {
        self.fetched.clear();
    }

    pub fn persist(&mut self, store: &dyn SecureStore, storage_path: &str, format: CacheFormat) {
        self.prune();
        let json = match format {
            CacheFormat::Stamps => match serde_json::to_string(&DiskCache {
                days: self.days.clone(),
                stamps: self.stamps.clone(),
            }) {
                Ok(j) => j,
                Err(e) => {
                    log::warn!("Не удалось сериализовать кеш дней: {:?}", e);
                    return;
                }
            },
            CacheFormat::LegacyMap => match serde_json::to_string(&self.days) {
                Ok(j) => j,
                Err(e) => {
                    log::warn!("Не удалось сериализовать кеш дней: {:?}", e);
                    return;
                }
            },
        };
        if let Err(e) = store.save(storage_path, FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE, &json) {
            log::warn!("Не удалось сохранить кеш дней: {:?}", e);
        }
    }

    fn prune(&mut self) {
        if self.days.len() <= MAX_DAYS {
            return;
        }
        let mut keys: Vec<String> = self.days.keys().cloned().collect();
        keys.sort(); // YYYY-MM-DD сортируется как дата
        let remove_n = self.days.len() - MAX_DAYS;
        for k in keys.into_iter().take(remove_n) {
            self.days.remove(&k);
            self.stamps.remove(&k);
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
            self.data
                .lock()
                .unwrap()
                .insert(f.to_string(), d.to_string());
            Ok(())
        }
        fn delete(&self, _p: &str, f: &str) {
            self.data.lock().unwrap().remove(f);
        }
    }

    #[test]
    fn reads_legacy_map_and_current_format() {
        let store = MemStore::default();
        store
            .save(
                "p",
                FILE_MARKS_CACHE,
                CONTEXT_MARKS_CACHE,
                r#"{"2026-01-01":"raw"}"#,
            )
            .unwrap();
        let cache = DayCache::load(&store, "p");
        assert_eq!(cache.get_raw("2026-01-01").as_deref(), Some("raw"));
        assert_eq!(cache.fetched_at("2026-01-01"), None);

        store
            .save(
                "p",
                FILE_MARKS_CACHE,
                CONTEXT_MARKS_CACHE,
                r#"{"days":{"2026-01-02":"raw2"},"stamps":{"2026-01-02":123}}"#,
            )
            .unwrap();
        let cache = DayCache::load(&store, "p");
        assert_eq!(cache.get_raw("2026-01-02").as_deref(), Some("raw2"));
        assert_eq!(cache.fetched_at("2026-01-02"), Some(123));
    }

    #[test]
    fn persists_in_selected_format() {
        let store = MemStore::default();
        let mut cache = DayCache::default();
        cache.put_mem("2026-01-01", "raw");
        cache.persist(&store, "p", CacheFormat::LegacyMap);
        let legacy = store
            .load("p", FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE)
            .unwrap();
        assert!(
            legacy.starts_with('{') && legacy.contains("2026-01-01") && !legacy.contains("stamps")
        );

        cache.persist(&store, "p", CacheFormat::Stamps);
        let current = store
            .load("p", FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE)
            .unwrap();
        assert!(current.contains("\"stamps\""));
    }
}
