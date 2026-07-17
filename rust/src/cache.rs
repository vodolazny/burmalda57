// Кеш дней дневника:
//  * в памяти — какие дни уже загружены за текущую сессию (чтобы не бить в сеть повторно);
//  * на диске (зашифрованно) — чтобы просмотренные дни открывались без интернета.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::crypto;

struct CacheState {
    storage_path: String,
    days: HashMap<String, String>, // дата "YYYY-MM-DD" -> сырой ответ diaryday
    fetched: HashSet<String>,      // даты, реально загруженные из сети в этой сессии
}

static STATE: Mutex<Option<CacheState>> = Mutex::new(None);

// Сколько дней держим в кеше 
const MAX_DAYS: usize = 120;

// Инициализация при входе: подхватываем сохранённый кеш с диска.
pub(crate) fn init(storage_path: &str) {
    let mut days = HashMap::new();
    if let Some(json) = crypto::load_marks_cache(storage_path) {
        match serde_json::from_str::<HashMap<String, String>>(&json) {
            Ok(map) => {
                log::info!("Кеш дней загружен с диска: {} дней", map.len());
                days = map;
            }
            Err(e) => log::warn!("Не удалось разобрать кеш дней: {:?}", e),
        }
    }
    *STATE.lock().unwrap() = Some(CacheState {
        storage_path: storage_path.to_string(),
        days,
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
        s.fetched.insert(date.to_string());
    }
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
fn prune(days: &mut HashMap<String, String>) {
    if days.len() <= MAX_DAYS {
        return;
    }
    let mut keys: Vec<String> = days.keys().cloned().collect();
    keys.sort(); // YYYY-MM-DD сортируется как дата
    let remove_n = days.len() - MAX_DAYS;
    for k in keys.into_iter().take(remove_n) {
        days.remove(&k); // удаляем самые старые
    }
}

// Сбрасываем текущий кеш дней на диск (зашифрованно).
pub(crate) fn persist() {
    let snapshot = {
        let mut g = STATE.lock().unwrap();
        match g.as_mut() {
            Some(s) => {
                prune(&mut s.days);
                (
                    s.storage_path.clone(),
                    serde_json::to_string(&s.days).ok(),
                )
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