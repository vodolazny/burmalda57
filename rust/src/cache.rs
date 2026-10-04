// Кеш дней дневника:
//  * в памяти — какие дни уже загружены за текущую сессию (чтобы не бить в сеть повторно);
//  * на диске (зашифрованно) — чтобы просмотренные дни открывались без интернета;
//  * для каждого дня храним время последней успешной загрузки — баннер сбоя
//    показывает «Данные от 14 сент, 23:16».
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::crypto;

struct CacheState {
    storage_path: String,
    days: HashMap<String, String>, // дата "YYYY-MM-DD" -> сырой ответ diaryday
    stamps: HashMap<String, i64>,  // дата -> unix-время последней загрузки (сек)
    fetched: HashSet<String>,      // даты, реально загруженные из сети в этой сессии
}

// Формат на диске. Раньше это была просто карта «дата -> ответ»; теперь рядом
// лежат отметки времени, поэтому читаем оба варианта.
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
    // ВАЖНО: старый формат первым — у него значения строки, и новый формат
    // (значения-объекты) в него не разберётся. Обратный порядок сломал бы
    // чтение старого кеша (он бы распознался как новый с пустыми полями).
    Legacy(HashMap<String, String>),
    Current(DiskCache),
}

static STATE: Mutex<Option<CacheState>> = Mutex::new(None);

// Сколько дней держим в кеше
const MAX_DAYS: usize = 120;

fn now_secs() -> i64 {
    chrono::Local::now().timestamp()
}

// Инициализация при входе: подхватываем сохранённый кеш с диска.
pub(crate) fn init(storage_path: &str) {
    let mut days = HashMap::new();
    let mut stamps = HashMap::new();
    if let Some(json) = crypto::load_marks_cache(storage_path) {
        match serde_json::from_str::<DiskCacheCompat>(&json) {
            Ok(DiskCacheCompat::Legacy(map)) => {
                log::info!("Кеш дней загружен с диска (старый формат): {} дней", map.len());
                days = map;
            }
            Ok(DiskCacheCompat::Current(c)) => {
                log::info!("Кеш дней загружен с диска: {} дней", c.days.len());
                days = c.days;
                stamps = c.stamps;
            }
            Err(e) => log::warn!("Не удалось разобрать кеш дней: {:?}", e),
        }
    }
    *STATE.lock().unwrap() = Some(CacheState {
        storage_path: storage_path.to_string(),
        days,
        stamps,
        fetched: HashSet::new(),
    });
}

// Путь приватного хранилища (для других кешей, напр. оценок).
pub(crate) fn storage_path() -> Option<String> {
    STATE.lock().unwrap().as_ref().map(|s| s.storage_path.clone())
}

// Сырой ответ за день (из памяти или подгруженный с диска), если есть.
pub(crate) fn get_raw(date: &str) -> Option<String> {
    let g = STATE.lock().unwrap();
    g.as_ref()?.days.get(date).cloned()
}

// Когда этот день последний раз успешно загружался (unix, сек).
pub(crate) fn fetched_at(date: &str) -> Option<i64> {
    let g = STATE.lock().unwrap();
    g.as_ref()?.stamps.get(date).copied()
}

// Загружали ли этот день из сети в текущей сессии.
pub(crate) fn is_fetched(date: &str) -> bool {
    let g = STATE.lock().unwrap();
    g.as_ref().map(|s| s.fetched.contains(date)).unwrap_or(false)
}

// Кладём свежий ответ в память и помечаем день как загруженный в этой сессии.
pub(crate) fn put_mem(date: &str, raw: &str) {
    let mut g = STATE.lock().unwrap();
    if let Some(s) = g.as_mut() {
        s.days.insert(date.to_string(), raw.to_string());
        s.stamps.insert(date.to_string(), now_secs());
        s.fetched.insert(date.to_string());
    }
}

// Полный сброс состояния при выходе из аккаунта.
pub(crate) fn reset() {
    *STATE.lock().unwrap() = None;
}

// Сбрасываем пометку «загружен в этой сессии» — заставит перезапросить день из сети.
// Копия на диске остаётся (для мгновенного показа/оффлайна).
pub(crate) fn invalidate(date: &str) {
    let mut g = STATE.lock().unwrap();
    if let Some(s) = g.as_mut() {
        s.fetched.remove(date);
    }
}

// Оставляем только последние MAX_DAYS дней (по дате).
fn prune(days: &mut HashMap<String, String>, stamps: &mut HashMap<String, i64>) {
    if days.len() <= MAX_DAYS {
        return;
    }
    let mut keys: Vec<String> = days.keys().cloned().collect();
    keys.sort(); // YYYY-MM-DD сортируется как дата
    let remove_n = days.len() - MAX_DAYS;
    for k in keys.into_iter().take(remove_n) {
        days.remove(&k); // удаляем самые старые
        stamps.remove(&k);
    }
}

// Сбрасываем текущий кеш дней на диск (зашифрованно).
pub(crate) fn persist() {
    let snapshot = {
        let mut g = STATE.lock().unwrap();
        match g.as_mut() {
            Some(s) => {
                prune(&mut s.days, &mut s.stamps);
                let disk = DiskCache {
                    days: s.days.clone(),
                    stamps: s.stamps.clone(),
                };
                (s.storage_path.clone(), serde_json::to_string(&disk).ok())
            }
            None => return,
        }
    };
    let (path, json) = snapshot;
    if let Some(json) = json {
        if let Err(e) = crypto::save_marks_cache(&path, &json) {
            log::warn!("Не удалось сохранить кеш дней: {:?}", e);
        }
    }
}
