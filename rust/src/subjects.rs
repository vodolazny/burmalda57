// Фильтр предметов в дневнике: пользователь может скрыть лишние предметы
// из расписания на день (например «Разговоры о важном»).
//
// Храним зашифрованно на устройстве, по аналогии с homework.rs / events.rs:
//   known  — все предметы, когда-либо встретившиеся в дневнике/оценках;
//   hidden — скрытые.
// Список known нужен именно на диске: иначе сразу после старта во вкладке
// профиля были бы только предметы открытого дня.

use std::collections::BTreeSet;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use slint::{ModelRc, VecModel};

use crate::crypto::{load_decrypted_file, save_encrypted_file};
use crate::{SubjectToggle, APP_WEAK};

const SUBJECTS_FILE: &str = ".subjects_filter";
const CONTEXT_SUBJECTS: &[u8] = b"burmalda57_subjects_filter_context_v1";

#[derive(Serialize, Deserialize, Default, Clone)]
struct Stored {
    #[serde(default)]
    known: BTreeSet<String>,
    #[serde(default)]
    hidden: BTreeSet<String>,
}

static STATE: Mutex<Option<Stored>> = Mutex::new(None);
static STORAGE: Mutex<Option<String>> = Mutex::new(None);

// Загрузка с диска — вызывать один раз на старте, из фонового потока
// (дешифрование ходит в Android Keystore).
pub(crate) fn init(storage_path: &str) {
    *STORAGE.lock().unwrap() = Some(storage_path.to_string());
    let stored = load_decrypted_file(storage_path, SUBJECTS_FILE, CONTEXT_SUBJECTS)
        .and_then(|json| serde_json::from_str::<Stored>(&json).ok())
        .unwrap_or_default();
    *STATE.lock().unwrap() = Some(stored);
    apply_to_ui();
}

fn persist() {
    let storage = match STORAGE.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    let guard = STATE.lock().unwrap();
    if let Some(state) = guard.as_ref() {
        match serde_json::to_string(state) {
            Ok(json) => {
                if let Err(e) =
                    save_encrypted_file(&storage, SUBJECTS_FILE, CONTEXT_SUBJECTS, &json)
                {
                    log::warn!("Не удалось сохранить фильтр предметов: {:?}", e);
                }
            }
            Err(e) => log::warn!("Не удалось сериализовать фильтр предметов: {:?}", e),
        }
    }
}

// Скрыт ли предмет (проверяется на каждом показе дня).
pub(crate) fn is_hidden(subject: &str) -> bool {
    let key = subject.trim();
    if key.is_empty() {
        return false;
    }
    STATE
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.hidden.contains(key))
        .unwrap_or(false)
}

// Запомнить встретившиеся предметы, чтобы они появились в списке настроек.
// Возвращает true, если список изменился (имеет смысл обновить UI).
pub(crate) fn note_subjects<I, S>(subjects: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut changed = false;
    {
        let mut guard = STATE.lock().unwrap();
        let state = guard.get_or_insert_with(Stored::default);
        for s in subjects {
            let name = s.as_ref().trim();
            if name.is_empty() {
                continue;
            }
            if state.known.insert(name.to_string()) {
                changed = true;
            }
        }
    }
    if changed {
        persist();
        apply_to_ui();
    }
    changed
}

// Вкл/выкл предмета из вкладки профиля (hidden = true — не показывать).
pub(crate) fn set_hidden(subject: &str, hidden: bool) {
    let key = subject.trim().to_string();
    if key.is_empty() {
        return;
    }
    {
        let mut guard = STATE.lock().unwrap();
        let state = guard.get_or_insert_with(Stored::default);
        state.known.insert(key.clone());
        if hidden {
            state.hidden.insert(key);
        } else {
            state.hidden.remove(&key);
        }
    }
    persist();
    apply_to_ui();
    // Сразу перерисовываем открытый день — без сети, из кеша.
    crate::diary::reapply_current_day();
}

// Сброс при выходе из аккаунта: новый ученик — другие предметы.
pub(crate) fn reset() {
    *STATE.lock().unwrap() = Some(Stored::default());
    persist();
    apply_to_ui();
}

// Список для UI: все известные предметы по алфавиту + флаг скрытия.
fn toggles() -> Vec<(String, bool)> {
    let guard = STATE.lock().unwrap();
    match guard.as_ref() {
        Some(state) => state
            .known
            .iter()
            .map(|s| (s.clone(), state.hidden.contains(s)))
            .collect(),
        None => Vec::new(),
    }
}

// Проброс списка в Slint (строго из его event loop).
pub(crate) fn apply_to_ui() {
    let items = toggles();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            let model: Vec<SubjectToggle> = items
                .into_iter()
                .map(|(subject, hidden)| SubjectToggle {
                    subject: subject.into(),
                    hidden,
                })
                .collect();
            ui.set_subject_toggles(ModelRc::new(VecModel::from(model)));
        }
    });
}
