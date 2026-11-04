// Состояние приложения и use-case'ы: дневник, оценки, итоговые, график,
// события/ДЗ/заметки. Core не знает про Slint и сеть: он отдаёт запросы
// (ApiRequest) адаптеру и принимает обратно сырые ответы.

use crate::api::{ApiRequest, FetchError, HttpResult};
use crate::cache::DayCache;
use crate::crypto::{
    CONTEXT_FINALS, CONTEXT_GRADES, CONTEXT_PERIODS, CONTEXT_SESSION, FILE_FINALS_CACHE,
    FILE_GRADES_CACHE, FILE_GRADES_NOTIFY, FILE_MARKS_CACHE, FILE_PERIODS, FILE_SESSION,
};
use crate::diary::{self, DiaryLessonData};
use crate::finals;
use crate::marks;
use crate::model::{
    ChartView, ChildInfo, DateView, DiaryView, LessonView, Mark, Period, RecentGradeView,
    SubjectFinalsView, SubjectGradesView, SubjectToggleView, UserSession,
};
use crate::ports::{CacheFormat, SecureStore};
use crate::stores::{EventsStore, HomeworkStore, NotesStore, SubjectsStore};
use std::collections::HashMap;

// ============================================================
//  Результаты операций для UI
// ============================================================
#[derive(Clone, Debug, Default)]
pub struct DiaryUpdate {
    pub date: Option<DateView>,
    pub view: Option<DiaryView>,
    /// None — не трогать спиннер; Some(false) — скрыть.
    pub loading: Option<bool>,
    /// Some("") — убрать баннер; Some(text) — показать.
    pub error: Option<String>,
    /// Подпись «Данные от …» рядом с ошибкой.
    pub stale: Option<String>,
    /// Обновлённый список предметов для вкладки профиля.
    pub toggles: Option<Vec<SubjectToggleView>>,
}

#[derive(Clone, Debug)]
pub struct DiaryFetch {
    pub request: ApiRequest,
    pub date: String,
    pub gen: u64,
    pub session_epoch: u64,
    pub cache_gen: u64,
}

#[derive(Clone, Debug)]
pub enum DiaryAction {
    Render(DiaryUpdate),
    Fetch {
        update: DiaryUpdate,
        fetch: DiaryFetch,
    },
}

#[derive(Clone, Debug, Default)]
pub struct MarksUpdate {
    pub periods: Option<Vec<String>>,
    pub selected: Option<i32>,
    pub subjects: Option<Vec<SubjectGradesView>>,
    pub loading: Option<bool>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MarksFetch {
    pub request: ApiRequest,
    pub range: String,
    pub gen: u64,
    pub had_cache: bool,
    pub session_epoch: u64,
    pub cache_gen: u64,
}

#[derive(Clone, Debug)]
pub enum MarksAction {
    Render(MarksUpdate),
    FetchPeriods {
        update: MarksUpdate,
        request: ApiRequest,
        gen: u64,
    },
    FetchMarks {
        update: MarksUpdate,
        fetch: MarksFetch,
    },
}

#[derive(Clone, Debug)]
pub struct ChartFetch {
    pub request: ApiRequest,
    pub gen: u64,
    pub index: usize,
    pub cache_gen: u64,
}

#[derive(Clone, Debug)]
pub enum ChartStep {
    Fetch(ChartFetch),
    Done(ChartView),
    /// Ответ относится к уже закрытому/заменённому графику.
    Ignored,
}

#[derive(Clone, Debug, Default)]
pub struct FinalsUpdate {
    pub subjects: Option<Vec<SubjectFinalsView>>,
    pub loading: Option<bool>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FinalsFetch {
    pub request: ApiRequest,
    pub gen: u64,
    pub had_cache: bool,
    pub session_epoch: u64,
}

#[derive(Clone, Debug)]
pub enum FinalsAction {
    Render(FinalsUpdate),
    Fetch {
        update: FinalsUpdate,
        fetch: FinalsFetch,
    },
}

#[derive(Clone, Debug)]
pub struct RecentFetch {
    pub request: ApiRequest,
    pub session_epoch: u64,
    pub gen: u64,
}

#[derive(Clone, Debug)]
pub enum RecentAction {
    Render(Vec<RecentGradeView>),
    Fetch(RecentFetch),
}

// Лента недавних оценок по дням дневника (историческая стратегия веба:
// идём назад по дням, пока не наберём достаточно оценок).
#[derive(Clone, Debug)]
pub struct RecentDiaryFetch {
    pub request: ApiRequest,
    pub date: String,
    pub label: String,
    pub gen: u64,
    pub cache_gen: u64,
}

#[derive(Clone, Debug)]
pub enum RecentDiaryAction {
    Fetch(RecentDiaryFetch),
    Done(Vec<RecentGradeView>),
    Ignored,
}

const RECENT_DIARY_MAX_BACK: i64 = 70;
const RECENT_DIARY_WANT: usize = 15;

#[derive(Clone, Debug)]
pub struct SubjectFilterUpdate {
    pub toggles: Vec<SubjectToggleView>,
    pub diary: Option<DiaryUpdate>,
}

// ============================================================
//  Состояние
// ============================================================
struct ChartJob {
    gen: u64,
    subject: String,
    periods: Vec<Period>,
    quarters: Vec<Vec<Mark>>,
}

struct RecentDiaryJob {
    gen: u64,
    offset: i64,
    grades: Vec<RecentGradeView>,
}

pub struct Core {
    storage_path: String,
    store: Box<dyn SecureStore>,
    cache_format: CacheFormat,
    session_epoch: u64,
    auth_generation: u64,
    cache_sequence: u64,
    diary_versions: HashMap<String, u64>,
    grade_versions: HashMap<String, u64>,
    recent_gen: u64,

    pub session: Option<UserSession>,
    pub demo: bool,
    pub current_date: Option<String>,

    // Дневник
    diary_cache: DayCache,
    diary_gen: u64,

    // Оценки по периодам
    marks_gen: u64,
    periods: Vec<Period>,
    grades_cache: Vec<(String, String)>, // "from..to" -> raw
    grades_fetched: Vec<String>,
    grades_loaded: bool,
    selected_period: usize,

    // Итоговые
    finals_gen: u64,
    finals_raw: Option<String>,
    finals_fetched: bool,
    finals_mode: i32,

    // График
    chart_gen: u64,
    chart_job: Option<ChartJob>,

    // Лента недавних оценок по дням
    recent_diary_gen: u64,
    recent_diary_job: Option<RecentDiaryJob>,

    // Локальные данные
    events: EventsStore,
    homework: HomeworkStore,
    notes: NotesStore,
    subjects: SubjectsStore,
}

impl Core {
    /// `store` должен быть готов к синхронным чтению/записи (на вебе —
    /// после init_keystore().await).
    pub fn new(
        storage_path: impl Into<String>,
        store: Box<dyn SecureStore>,
        cache_format: CacheFormat,
    ) -> Self {
        let storage_path = storage_path.into();
        let events = EventsStore::load(store.as_ref(), &storage_path);
        let homework = HomeworkStore::load(store.as_ref(), &storage_path);
        let notes = NotesStore::load(store.as_ref(), &storage_path);
        let subjects = SubjectsStore::load(store.as_ref(), &storage_path);
        Self {
            storage_path,
            store,
            cache_format,
            session_epoch: 0,
            auth_generation: 0,
            cache_sequence: 0,
            diary_versions: HashMap::new(),
            grade_versions: HashMap::new(),
            recent_gen: 0,
            session: None,
            demo: false,
            current_date: None,
            diary_cache: DayCache::default(),
            diary_gen: 0,
            marks_gen: 0,
            periods: Vec::new(),
            grades_cache: Vec::new(),
            grades_fetched: Vec::new(),
            grades_loaded: false,
            selected_period: 0,
            finals_gen: 0,
            finals_raw: None,
            finals_fetched: false,
            finals_mode: 0,
            chart_gen: 0,
            chart_job: None,
            recent_diary_gen: 0,
            recent_diary_job: None,
            events,
            homework,
            notes,
            subjects,
        }
    }

    pub fn storage_path(&self) -> &str {
        &self.storage_path
    }

    pub fn store(&self) -> &dyn SecureStore {
        self.store.as_ref()
    }

    pub fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    /// Invalidated by another login/selection, cancellation or session change.
    pub fn begin_auth(&mut self) -> u64 {
        self.auth_generation = self.auth_generation.wrapping_add(1);
        self.auth_generation
    }

    pub fn auth_is_current(&self, generation: u64) -> bool {
        self.auth_generation == generation
    }

    fn next_cache_generation(&mut self) -> u64 {
        self.cache_sequence = self.cache_sequence.wrapping_add(1);
        self.cache_sequence
    }

    // ========================================================
    //  Сессия
    // ========================================================
    pub fn save_session(&self, session: &UserSession) -> Result<(), String> {
        let json = serde_json::to_string(session).map_err(|e| e.to_string())?;
        self.store
            .save(&self.storage_path, FILE_SESSION, CONTEXT_SESSION, &json)
            .map_err(|e| format!("Не удалось сохранить сессию: {e}"))
    }

    /// Загрузить сессию и кеш дней. Вызывать один раз при старте.
    pub fn restore_session(&mut self) -> Option<UserSession> {
        let json = self
            .store
            .load(&self.storage_path, FILE_SESSION, CONTEXT_SESSION)?;
        let session: UserSession = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Сессия не разобралась: {:?}", e);
                return None;
            }
        };
        self.reset_learning_state();
        self.session = Some(session.clone());
        self.demo = false;
        self.migrate_local_keys(&session.user_guid);
        self.diary_cache = DayCache::load(self.store.as_ref(), &self.storage_path);
        Some(session)
    }

    /// Принять сессию после логина/переключения. `persist` = сохранить на диск.
    pub fn adopt_session(&mut self, session: UserSession, persist: bool) -> Result<(), String> {
        if persist {
            self.save_session(&session)?;
        }
        self.reset_learning_state();
        self.session = Some(session);
        self.demo = false;
        Ok(())
    }

    /// Вход: сохранить сессию и при смене ученика сбросить чужие серверные кеши.
    pub fn adopt_login_session(&mut self, session: UserSession) -> Result<(), String> {
        let switched = self.demo
            || self
                .session
                .as_ref()
                .map(|s| s.user_guid != session.user_guid)
                .unwrap_or(true);
        self.save_session(&session)?;
        self.reset_learning_state();
        if switched {
            self.delete_server_caches();
        } else {
            self.diary_cache = DayCache::load(self.store.as_ref(), &self.storage_path);
        }
        self.demo = false;
        self.session = Some(session);
        Ok(())
    }

    /// Демо-режим: сменить ученика без записи на диск.
    pub fn switch_demo_session(&mut self, session: UserSession) {
        self.demo = true;
        self.session = Some(session);
        self.reset_learning_state();
    }

    /// Переключение ребёнка: новая сессия на диск + сброс серверных кешей.
    pub fn apply_switched_session(&mut self, session: UserSession) -> Result<(), String> {
        self.save_session(&session)?;
        self.session = Some(session);
        self.demo = false;
        self.reset_learning_state();
        self.delete_server_caches();
        Ok(())
    }

    pub fn session(&self) -> Option<&UserSession> {
        self.session.as_ref()
    }

    pub fn session_clone(&self) -> Option<UserSession> {
        self.session.clone()
    }

    pub fn enter_demo(&mut self) -> UserSession {
        let session = demo_session();
        self.reset_learning_state();
        self.demo = true;
        self.session = Some(session.clone());
        session
    }

    /// Выход: сессия и серверные кеши удаляются, локальные события/ДЗ/заметки
    /// остаются. Платформенные вещи (аватар, воркер, UI) сбрасывает адаптер.
    pub fn logout(&mut self) {
        self.session = None;
        self.demo = false;
        self.reset_learning_state();
        self.delete_server_caches();
        self.store.delete(&self.storage_path, FILE_SESSION);
        self.subjects.reset(self.store.as_ref(), &self.storage_path);
    }

    fn reset_learning_state(&mut self) {
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.begin_auth();
        self.diary_versions.clear();
        self.grade_versions.clear();
        self.recent_gen = self.recent_gen.wrapping_add(1);
        self.diary_cache = DayCache::default();
        self.diary_gen = self.diary_gen.wrapping_add(1);
        self.marks_gen = self.marks_gen.wrapping_add(1);
        self.periods.clear();
        self.grades_cache.clear();
        self.grades_fetched.clear();
        self.grades_loaded = false;
        self.selected_period = 0;
        self.finals_gen = self.finals_gen.wrapping_add(1);
        self.finals_raw = None;
        self.finals_fetched = false;
        self.finals_mode = 0;
        self.chart_gen = self.chart_gen.wrapping_add(1);
        self.chart_job = None;
        self.recent_diary_gen = self.recent_diary_gen.wrapping_add(1);
        self.recent_diary_job = None;
    }

    fn migrate_local_keys(&mut self, guid: &str) {
        // Only a saved session identifies the owner of legacy unscoped records.
        let prefix = format!("student:{guid}|");
        self.notes
            .migrate_legacy(self.store.as_ref(), &self.storage_path, &prefix);
        self.homework
            .migrate_legacy(self.store.as_ref(), &self.storage_path, &prefix);
    }

    fn scoped_lesson_key(&self, date: &str, number: i32, subject: &str) -> String {
        let guid = self
            .session
            .as_ref()
            .map(|s| s.user_guid.as_str())
            .unwrap_or_default();
        format!("student:{guid}|{}", NotesStore::key(date, number, subject))
    }

    fn owns_lesson_key(&self, key: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| key.starts_with(&format!("student:{}|", s.user_guid)))
    }

    fn delete_server_caches(&self) {
        for f in [
            FILE_MARKS_CACHE,
            FILE_GRADES_CACHE,
            FILE_PERIODS,
            FILE_FINALS_CACHE,
            FILE_GRADES_NOTIFY,
        ] {
            self.store.delete(&self.storage_path, f);
        }
    }

    // ========================================================
    //  Дневник
    // ========================================================
    pub fn set_current_date(&mut self, date: String) {
        self.current_date = Some(date);
    }

    pub fn pick_date(&mut self, year: i32, month: i32, day: i32) -> Option<DiaryAction> {
        self.current_date = Some(format!("{:04}-{:02}-{:02}", year, month, day));
        self.refresh_diary(0)
    }

    pub fn refresh_diary(&mut self, delta: i64) -> Option<DiaryAction> {
        let date = {
            let current = self.current_date.clone().unwrap_or_else(diary::today);
            let next = if delta == 0 {
                current
            } else {
                diary::shift_date(&current, delta)
            };
            self.current_date = Some(next.clone());
            next
        };
        self.session.as_ref()?;

        if self.demo {
            let lessons = diary::demo_lessons(&date);
            let update = self.build_diary_update(&date, lessons);
            return Some(DiaryAction::Render(DiaryUpdate {
                loading: Some(false),
                error: Some(String::new()),
                stale: Some(String::new()),
                ..update
            }));
        }

        self.diary_gen = self.diary_gen.wrapping_add(1);
        let gen = self.diary_gen;

        if self.diary_cache.is_fetched(&date) {
            if let Some(raw) = self.diary_cache.get_raw(&date) {
                let update = self.build_diary_update(&date, diary::parse(&raw));
                return Some(DiaryAction::Render(DiaryUpdate {
                    loading: Some(false),
                    error: Some(String::new()),
                    ..update
                }));
            }
        }

        let cached = self
            .diary_cache
            .get_raw(&date)
            .and_then(|raw| diary::parse_checked(&raw).ok());
        // Always replace the previous day's lessons, even when only local events exist.
        let mut update = self.build_diary_update(&date, cached.unwrap_or_default());
        update.loading = Some(true);
        update.error = Some(String::new());
        update.stale = Some(String::new());

        let session = self.session.clone()?;
        let cache_gen = self.next_cache_generation();
        Some(DiaryAction::Fetch {
            update,
            fetch: DiaryFetch {
                request: diary::request(&session, &date),
                date,
                gen,
                session_epoch: self.session_epoch,
                cache_gen,
            },
        })
    }

    pub fn force_refresh_diary(&mut self) -> Option<DiaryAction> {
        let date = self.current_date.clone().unwrap_or_else(diary::today);
        self.diary_cache.invalidate(&date);
        self.refresh_diary(0)
    }

    /// Перерисовать текущий день из кеша (после событий/фильтра предметов).
    pub fn reapply_current_day(&mut self) -> Option<DiaryUpdate> {
        let date = self.current_date.clone().unwrap_or_else(diary::today);
        let lessons = if self.demo {
            diary::demo_lessons(&date)
        } else {
            self.diary_cache
                .get_raw(&date)
                .map(|raw| diary::parse(&raw))
                .unwrap_or_default()
        };
        Some(self.build_diary_update(&date, lessons))
    }

    pub fn complete_diary(
        &mut self,
        fetch: &DiaryFetch,
        result: HttpResult,
    ) -> Option<DiaryUpdate> {
        if self.session.is_none() || fetch.session_epoch != self.session_epoch {
            return None;
        }
        let latest = self.diary_gen == fetch.gen;
        let raw = crate::api::post_raw(result);
        match raw {
            Ok(raw) => match diary::parse_checked(&raw) {
                Ok(lessons) => {
                    // A background scan can share this date. Order cache writes by
                    // completed responses without suppressing the visible request.
                    let lessons = if self
                        .diary_versions
                        .get(&fetch.date)
                        .is_none_or(|&v| fetch.cache_gen >= v)
                    {
                        self.diary_versions
                            .insert(fetch.date.clone(), fetch.cache_gen);
                        self.diary_cache.put_mem(&fetch.date, &raw);
                        self.diary_cache.persist(
                            self.store.as_ref(),
                            &self.storage_path,
                            self.cache_format,
                        );
                        lessons
                    } else {
                        self.diary_cache
                            .get_raw(&fetch.date)
                            .and_then(|raw| diary::parse_checked(&raw).ok())
                            .unwrap_or(lessons)
                    };
                    self.diary_cache.mark_fetched(&fetch.date);
                    if latest {
                        let update = self.build_diary_update(&fetch.date, lessons);
                        Some(DiaryUpdate {
                            loading: Some(false),
                            error: Some(String::new()),
                            stale: Some(String::new()),
                            ..update
                        })
                    } else {
                        None
                    }
                }
                Err(e) => {
                    if latest {
                        Some(error_diary_update(&e, &fetch.date, &self.diary_cache))
                    } else {
                        None
                    }
                }
            },
            Err(e) => {
                if latest {
                    Some(error_diary_update(&e, &fetch.date, &self.diary_cache))
                } else {
                    None
                }
            }
        }
    }

    fn build_diary_update(&mut self, date: &str, lessons: Vec<DiaryLessonData>) -> DiaryUpdate {
        // Свои события → в «уроки» этого дня.
        let event_items: Vec<LessonView> = self
            .events
            .for_date(date)
            .into_iter()
            .map(|e| LessonView {
                number: 0,
                time: if e.end.trim().is_empty() {
                    e.start.clone()
                } else {
                    format!("{} – {}", e.start, e.end)
                },
                subject: e.name,
                room: String::new(),
                homework: String::new(),
                hw_hint: String::new(),
                topic: String::new(),
                teacher: String::new(),
                mark: String::new(),
                mark_value: 0,
                absence: String::new(),
                grade_type: String::new(),
                start: e.start,
                is_event: true,
                event_id: e.id,
                hw_done: false,
                hw_key: String::new(),
                note: String::new(),
                note_key: String::new(),
            })
            .collect();

        // Запоминаем все предметы дня (для вкладки «Предметы»).
        let subjects_changed = {
            let store = self.store.as_ref();
            let path = self.storage_path.as_str();
            self.subjects
                .note_subjects(store, path, lessons.iter().map(|l| l.subject.as_str()))
        };

        let mut items: Vec<LessonView> = lessons
            .into_iter()
            .filter(|l| !self.subjects.is_hidden(&l.subject))
            .map(|l| {
                let hw_key = if l.homework.is_empty() {
                    String::new()
                } else {
                    self.scoped_lesson_key(date, l.number, &l.subject)
                };
                let hw_done = self.homework.is_done(&hw_key);
                let note_key = self.scoped_lesson_key(date, l.number, &l.subject);
                let note = self.notes.get(&note_key);
                LessonView {
                    number: l.number,
                    time: l.time,
                    subject: l.subject,
                    room: l.room,
                    homework: l.homework,
                    hw_hint: l.hw_hint,
                    topic: l.topic,
                    teacher: l.teacher,
                    mark: l.mark,
                    mark_value: l.mark_value,
                    absence: l.absence,
                    grade_type: l.grade_type,
                    start: l.start,
                    is_event: false,
                    event_id: String::new(),
                    hw_done,
                    hw_key,
                    note,
                    note_key,
                }
            })
            .collect();

        items.extend(event_items);
        items.sort_by(|a, b| {
            let ka = if a.start.trim().is_empty() {
                "99:99"
            } else {
                a.start.trim()
            };
            let kb = if b.start.trim().is_empty() {
                "99:99"
            } else {
                b.start.trim()
            };
            ka.cmp(kb).then(a.number.cmp(&b.number))
        });

        let dv = date_view(date);
        DiaryUpdate {
            date: Some(dv.clone()),
            view: Some(DiaryView {
                date: dv.date,
                year: dv.year,
                month: dv.month,
                day: dv.day,
                lessons: items,
            }),
            toggles: if subjects_changed {
                Some(self.subjects.toggles())
            } else {
                None
            },
            ..Default::default()
        }
    }

    pub fn add_event_current(&mut self, name: &str, start: &str, end: &str) -> Option<DiaryUpdate> {
        let date = self.current_date.clone().unwrap_or_else(diary::today);
        self.events.add(
            self.store.as_ref(),
            &self.storage_path,
            &date,
            name,
            start,
            end,
        );
        self.reapply_current_day()
    }

    pub fn delete_event_current(&mut self, id: &str) -> Option<DiaryUpdate> {
        let date = self.current_date.clone().unwrap_or_else(diary::today);
        self.events
            .delete(self.store.as_ref(), &self.storage_path, &date, id);
        self.reapply_current_day()
    }

    pub fn toggle_homework(&mut self, key: &str, done: bool) {
        if !self.owns_lesson_key(key) {
            return;
        }
        self.homework
            .set_done(self.store.as_ref(), &self.storage_path, key, done);
    }

    pub fn save_note(&mut self, key: &str, text: &str) {
        if !self.owns_lesson_key(key) {
            return;
        }
        self.notes
            .set(self.store.as_ref(), &self.storage_path, key, text);
    }

    pub fn subject_toggles(&self) -> Vec<SubjectToggleView> {
        self.subjects.toggles()
    }

    pub fn set_subject_hidden(&mut self, subject: &str, hidden: bool) -> SubjectFilterUpdate {
        self.subjects
            .set_hidden(self.store.as_ref(), &self.storage_path, subject, hidden);
        SubjectFilterUpdate {
            toggles: self.subjects.toggles(),
            diary: self.reapply_current_day(),
        }
    }

    // ========================================================
    //  Оценки по периодам
    // ========================================================
    pub fn init_marks(&mut self) -> Option<MarksAction> {
        if self.demo {
            self.periods = marks::demo_periods();
            let names: Vec<String> = self.periods.iter().map(|p| p.name.clone()).collect();
            self.selected_period = 2;
            return Some(MarksAction::Render(MarksUpdate {
                periods: Some(names),
                selected: Some(2),
                subjects: Some(marks::subject_views(marks::demo_subject_marks())),
                loading: Some(false),
                error: Some(String::new()),
            }));
        }
        self.session.as_ref()?;
        if !self.grades_loaded {
            self.load_grades_disk();
            self.grades_loaded = true;
        }
        self.marks_gen = self.marks_gen.wrapping_add(1);
        let session = self.session.clone()?;
        Some(MarksAction::FetchPeriods {
            update: MarksUpdate {
                loading: Some(true),
                ..Default::default()
            },
            request: marks::periods_request(&session),
            gen: self.marks_gen,
        })
    }

    pub fn complete_periods(&mut self, gen: u64, result: HttpResult) -> Option<MarksAction> {
        let latest = self.marks_gen == gen;
        if !latest || self.session.is_none() {
            return None;
        }
        let raw = match crate::api::post_raw(result).and_then(|raw| {
            let periods = marks::parse_periods_checked(&raw)?;
            if periods.is_empty() {
                return Err(FetchError::BadData);
            }
            Ok(raw)
        }) {
            Ok(raw) => {
                self.persist_periods(&raw);
                raw
            }
            Err(e) => match self.load_periods_disk() {
                Some(cached) => {
                    log::info!("Периоды из офлайн-кеша ({})", e.message());
                    cached
                }
                None => {
                    if !latest {
                        return None;
                    }
                    return Some(MarksAction::Render(MarksUpdate {
                        loading: Some(false),
                        error: Some(e.message()),
                        ..Default::default()
                    }));
                }
            },
        };

        let periods = marks::parse_periods(&raw);
        if periods.is_empty() {
            if !latest {
                return None;
            }
            return Some(MarksAction::Render(MarksUpdate {
                loading: Some(false),
                error: Some("Не удалось загрузить четверти.".into()),
                ..Default::default()
            }));
        }

        let idx = marks::current_period_index(&periods);
        let names: Vec<String> = periods.iter().map(|p| p.name.clone()).collect();
        self.periods = periods;
        self.selected_period = idx;

        let base = MarksUpdate {
            periods: Some(names),
            selected: Some(idx as i32),
            ..Default::default()
        };
        if !latest {
            return None;
        }
        Some(self.load_marks_action(idx, base))
    }

    pub fn select_period(&mut self, idx: i32) -> Option<MarksAction> {
        if idx < 0 {
            return None;
        }
        let idx = idx as usize;
        self.selected_period = idx;
        let base = MarksUpdate {
            selected: Some(idx as i32),
            ..Default::default()
        };
        if self.demo {
            return Some(MarksAction::Render(MarksUpdate {
                selected: Some(idx as i32),
                subjects: Some(marks::subject_views(marks::demo_subject_marks_for_period(
                    idx,
                ))),
                loading: Some(false),
                error: Some(String::new()),
                ..Default::default()
            }));
        }
        self.session.as_ref()?;
        Some(self.load_marks_action(idx, base))
    }

    fn load_marks_action(&mut self, idx: usize, base: MarksUpdate) -> MarksAction {
        let Some(session) = self.session.clone() else {
            return MarksAction::Render(base);
        };
        let Some(period) = self.periods.get(idx).cloned() else {
            return MarksAction::Render(base);
        };
        let from = marks::to_iso(&period.from);
        let to = marks::to_iso(&period.to);
        let range = format!("{}..{}", from, to);

        self.marks_gen = self.marks_gen.wrapping_add(1);
        let gen = self.marks_gen;

        if self.is_fetched(&range) {
            if let Some(raw) = self.cache_get(&range) {
                return MarksAction::Render(MarksUpdate {
                    subjects: Some(marks::subject_views(marks::parse_marks(&raw))),
                    loading: Some(false),
                    error: Some(String::new()),
                    ..base
                });
            }
        }

        let cached = self
            .cache_get(&range)
            .filter(|raw| marks::parse_marks_checked(raw).is_ok());
        let mut update = base;
        if let Some(raw) = &cached {
            update.subjects = Some(marks::subject_views(marks::parse_marks(raw)));
            update.error = Some(String::new());
        }
        update.loading = Some(true);
        if cached.is_none() {
            update.subjects = Some(Vec::new());
            update.error = Some(String::new());
        }
        let cache_gen = self.next_cache_generation();

        MarksAction::FetchMarks {
            update,
            fetch: MarksFetch {
                request: marks::marks_request(&session, &from, &to),
                range,
                gen,
                had_cache: cached.is_some(),
                session_epoch: self.session_epoch,
                cache_gen,
            },
        }
    }

    pub fn complete_marks(
        &mut self,
        fetch: &MarksFetch,
        result: HttpResult,
    ) -> Option<MarksUpdate> {
        if self.session.is_none() || fetch.session_epoch != self.session_epoch {
            return None;
        }
        let latest = self.marks_gen == fetch.gen;
        let raw = crate::api::post_raw(result).and_then(|raw| {
            marks::parse_marks_checked(&raw)?;
            Ok(raw)
        });
        match raw {
            Ok(raw) => {
                let raw = if self
                    .grade_versions
                    .get(&fetch.range)
                    .is_none_or(|&v| fetch.cache_gen >= v)
                {
                    self.grade_versions
                        .insert(fetch.range.clone(), fetch.cache_gen);
                    self.cache_put(&fetch.range, &raw);
                    self.mark_fetched(&fetch.range);
                    self.persist_grades();
                    raw
                } else {
                    self.cache_get(&fetch.range).unwrap_or(raw)
                };
                if latest {
                    Some(MarksUpdate {
                        subjects: Some(marks::subject_views(marks::parse_marks(&raw))),
                        loading: Some(false),
                        error: Some(String::new()),
                        ..Default::default()
                    })
                } else {
                    None
                }
            }
            Err(e) => {
                if latest {
                    let mut update = MarksUpdate {
                        loading: Some(false),
                        ..Default::default()
                    };
                    if !fetch.had_cache {
                        update.error = Some(e.message());
                    }
                    Some(update)
                } else {
                    None
                }
            }
        }
    }

    // ========================================================
    //  График
    // ========================================================
    pub fn open_chart(&mut self, subject: &str) -> ChartStep {
        self.chart_gen = self.chart_gen.wrapping_add(1);
        let gen = self.chart_gen;
        self.chart_job = None;

        if self.demo {
            return ChartStep::Done(marks::demo_chart(subject));
        }
        let Some(session) = self.session.clone() else {
            return ChartStep::Done(ChartView::default());
        };
        if self.periods.is_empty() {
            return ChartStep::Done(ChartView::default());
        }

        let periods = self.periods.clone();
        let mut quarters: Vec<Vec<Mark>> = vec![Vec::new(); periods.len()];
        let mut next: Option<usize> = None;
        for (i, p) in periods.iter().enumerate() {
            let from = marks::to_iso(&p.from);
            let to = marks::to_iso(&p.to);
            let range = format!("{}..{}", from, to);
            match self.cache_get(&range) {
                Some(raw) => quarters[i] = marks_for_subject(&raw, subject),
                None => {
                    if next.is_none() {
                        next = Some(i);
                    }
                }
            }
        }

        self.chart_job = Some(ChartJob {
            gen,
            subject: subject.to_string(),
            periods: periods.clone(),
            quarters: quarters.clone(),
        });

        match next {
            Some(index) => {
                ChartStep::Fetch(self.chart_fetch(&session, &periods[index], gen, index))
            }
            None => ChartStep::Done(marks::build_chart_view(&periods, &quarters)),
        }
    }

    fn chart_fetch(
        &mut self,
        session: &UserSession,
        period: &Period,
        gen: u64,
        index: usize,
    ) -> ChartFetch {
        ChartFetch {
            request: quarter_request(session, period),
            gen,
            index,
            cache_gen: self.next_cache_generation(),
        }
    }

    pub fn chart_supply(&mut self, fetch: &ChartFetch, result: HttpResult) -> ChartStep {
        let (gen, index) = (fetch.gen, fetch.index);
        let Some(job) = self.chart_job.as_ref() else {
            return ChartStep::Ignored;
        };
        if job.gen != gen || index >= job.quarters.len() {
            return ChartStep::Ignored;
        }
        let periods = job.periods.clone();
        let subject = job.subject.clone();

        let raw = crate::api::post_raw(result).and_then(|raw| {
            marks::parse_marks_checked(&raw)?;
            Ok(raw)
        });
        match raw {
            Ok(raw) => {
                let range = format!(
                    "{}..{}",
                    marks::to_iso(&periods[index].from),
                    marks::to_iso(&periods[index].to)
                );
                let raw = if self
                    .grade_versions
                    .get(&range)
                    .is_none_or(|&v| fetch.cache_gen >= v)
                {
                    self.grade_versions.insert(range.clone(), fetch.cache_gen);
                    self.cache_put(&range, &raw);
                    self.mark_fetched(&range);
                    self.persist_grades();
                    raw
                } else {
                    self.cache_get(&range).unwrap_or(raw)
                };
                if let Some(job) = self.chart_job.as_mut() {
                    job.quarters[index] = marks_for_subject(&raw, &subject);
                }
            }
            Err(e) => {
                log::info!("График: четверть {} не получена: {}", index, e.message());
                if let Some(job) = self.chart_job.as_mut() {
                    job.quarters[index] = Vec::new();
                }
            }
        }

        let Some(job) = self.chart_job.as_ref() else {
            return ChartStep::Ignored;
        };
        // Ищем следующую четверть без кеша (пустая из-за ошибки тоже
        // попадётся, но повторно её не запрашиваем — как в прежней версии).
        let mut next = None;
        for (i, p) in job.periods.iter().enumerate() {
            if i == index || !job.quarters[i].is_empty() {
                continue;
            }
            let range = format!("{}..{}", marks::to_iso(&p.from), marks::to_iso(&p.to));
            if self.cache_get(&range).is_none() && i > index {
                next = Some(i);
                break;
            }
        }

        if let Some(next_index) = next {
            if let Some(session) = self.session.clone() {
                let period = job.periods[next_index].clone();
                return ChartStep::Fetch(self.chart_fetch(&session, &period, gen, next_index));
            }
        }

        let job = self.chart_job.take().unwrap();
        ChartStep::Done(marks::build_chart_view(&job.periods, &job.quarters))
    }

    // ========================================================
    //  Итоговые оценки
    // ========================================================
    pub fn init_finals(&mut self) -> Option<FinalsUpdate> {
        self.session.as_ref()?;
        if self.demo {
            return Some(FinalsUpdate {
                subjects: Some(finals::finals_views(finals::demo_finals())),
                loading: Some(false),
                error: Some(String::new()),
            });
        }
        if self.finals_raw.is_none() {
            if let Some(raw) =
                self.store
                    .load(&self.storage_path, FILE_FINALS_CACHE, CONTEXT_FINALS)
            {
                log::info!("Итоговые: кеш загружен с диска");
                if finals::parse_finals_checked(&raw).is_ok() {
                    self.finals_raw = Some(raw);
                }
            }
        }
        self.finals_raw.as_ref().map(|raw| FinalsUpdate {
            subjects: Some(finals::finals_views(finals::parse_finals(raw))),
            loading: None,
            error: None,
        })
    }

    pub fn select_finals_mode(&mut self, mode: i32) -> Option<FinalsAction> {
        self.finals_mode = mode;
        if mode == 1 {
            self.load_finals(false)
        } else {
            None
        }
    }

    pub fn finals_mode(&self) -> i32 {
        self.finals_mode
    }

    pub fn load_finals(&mut self, force: bool) -> Option<FinalsAction> {
        if self.demo {
            return Some(FinalsAction::Render(FinalsUpdate {
                subjects: Some(finals::finals_views(finals::demo_finals())),
                loading: Some(false),
                error: Some(String::new()),
            }));
        }
        let session = self.session.clone()?;

        let cached = self.finals_raw.clone();
        let mut update = FinalsUpdate {
            subjects: Some(Vec::new()),
            ..Default::default()
        };
        if let Some(raw) = &cached {
            update.subjects = Some(finals::finals_views(finals::parse_finals(raw)));
        }

        if self.finals_fetched && !force {
            return Some(FinalsAction::Render(FinalsUpdate {
                loading: Some(false),
                error: Some(String::new()),
                ..update
            }));
        }

        update.loading = Some(true);
        self.finals_gen = self.finals_gen.wrapping_add(1);
        Some(FinalsAction::Fetch {
            update,
            fetch: FinalsFetch {
                request: finals::request(&session),
                gen: self.finals_gen,
                had_cache: cached.is_some(),
                session_epoch: self.session_epoch,
            },
        })
    }

    pub fn complete_finals(
        &mut self,
        fetch: &FinalsFetch,
        result: HttpResult,
    ) -> Option<FinalsUpdate> {
        let latest = self.finals_gen == fetch.gen;
        if !latest || fetch.session_epoch != self.session_epoch || self.session.is_none() {
            return None;
        }
        let raw = crate::api::post_raw(result).and_then(|raw| {
            finals::parse_finals_checked(&raw)?;
            Ok(raw)
        });
        match raw {
            Ok(raw) => {
                self.finals_raw = Some(raw.clone());
                self.finals_fetched = true;
                if let Err(e) =
                    self.store
                        .save(&self.storage_path, FILE_FINALS_CACHE, CONTEXT_FINALS, &raw)
                {
                    log::warn!("Не удалось сохранить кеш итоговых: {:?}", e);
                }
                if latest {
                    Some(FinalsUpdate {
                        subjects: Some(finals::finals_views(finals::parse_finals(&raw))),
                        loading: Some(false),
                        error: Some(String::new()),
                    })
                } else {
                    None
                }
            }
            Err(e) => {
                if latest {
                    Some(FinalsUpdate {
                        loading: Some(false),
                        error: if fetch.had_cache {
                            None
                        } else {
                            Some(e.message())
                        },
                        ..Default::default()
                    })
                } else {
                    None
                }
            }
        }
    }

    // ========================================================
    //  Лента недавних оценок
    // ========================================================
    pub fn recent_grades_start(&mut self) -> Option<RecentAction> {
        if self.demo {
            return Some(RecentAction::Render(diary::demo_recent()));
        }
        let session = self.session.clone()?;
        let (from, to) = diary::recent_range();
        self.recent_gen = self.recent_gen.wrapping_add(1);
        Some(RecentAction::Fetch(RecentFetch {
            request: marks::marks_request(&session, &from, &to),
            gen: self.recent_gen,
            session_epoch: self.session_epoch,
        }))
    }

    pub fn complete_recent_grades(
        &mut self,
        fetch: &RecentFetch,
        result: HttpResult,
    ) -> Option<Vec<RecentGradeView>> {
        if fetch.session_epoch != self.session_epoch
            || fetch.gen != self.recent_gen
            || self.session.is_none()
        {
            return None;
        }
        let raw = match crate::api::post_raw(result).and_then(|raw| {
            marks::parse_marks_checked(&raw)?;
            Ok(raw)
        }) {
            Ok(raw) => raw,
            Err(e) => {
                log::info!("Лента недавних оценок не обновлена: {}", e.message());
                return None;
            }
        };
        let recent = diary::build_recent(&marks::parse_marks(&raw));
        log::info!("Лента недавних оценок: собрано {}", recent.len());
        Some(recent)
    }

    /// Лента недавних оценок по дням дневника (стратегия веба): старт прохода
    /// назад по календарю до набора RECENT_DIARY_WANT оценок.
    pub fn recent_diary_start(&mut self) -> RecentDiaryAction {
        if self.demo {
            return RecentDiaryAction::Done(diary::demo_recent());
        }
        self.recent_diary_gen = self.recent_diary_gen.wrapping_add(1);
        let gen = self.recent_diary_gen;
        if self.session.is_none() {
            self.recent_diary_job = None;
            return RecentDiaryAction::Done(Vec::new());
        }
        self.recent_diary_job = Some(RecentDiaryJob {
            gen,
            offset: 0,
            grades: Vec::new(),
        });
        self.recent_diary_next()
    }

    pub fn recent_diary_supply(
        &mut self,
        fetch: &RecentDiaryFetch,
        result: HttpResult,
    ) -> RecentDiaryAction {
        let Some(job) = self.recent_diary_job.as_ref() else {
            return RecentDiaryAction::Ignored;
        };
        if job.gen != fetch.gen {
            return RecentDiaryAction::Ignored;
        }

        match crate::api::post_raw(result).and_then(|raw| {
            diary::parse_checked(&raw)?;
            Ok(raw)
        }) {
            Ok(raw) => {
                let raw = if self
                    .diary_versions
                    .get(&fetch.date)
                    .is_none_or(|&v| fetch.cache_gen >= v)
                {
                    self.diary_versions
                        .insert(fetch.date.clone(), fetch.cache_gen);
                    self.diary_cache.put_mem_no_mark(&fetch.date, &raw);
                    raw
                } else {
                    self.diary_cache.get_raw(&fetch.date).unwrap_or(raw)
                };
                let grades: Vec<RecentGradeView> = diary::parse(&raw)
                    .into_iter()
                    .filter(|l| l.mark_value > 0)
                    .map(|l| RecentGradeView {
                        subject: l.subject,
                        mark: l.mark,
                        mark_val: l.mark_value,
                        date: fetch.label.clone(),
                    })
                    .collect();
                if let Some(job) = self.recent_diary_job.as_mut() {
                    job.grades.extend(grades);
                    job.offset += 1;
                }
            }
            Err(e) => {
                log::info!("Лента: обход остановлен: {}", e.message());
                return self.finish_recent_diary();
            }
        }
        self.recent_diary_next()
    }

    fn recent_diary_next(&mut self) -> RecentDiaryAction {
        let Some(job) = self.recent_diary_job.as_ref() else {
            return RecentDiaryAction::Ignored;
        };
        if job.offset >= RECENT_DIARY_MAX_BACK || job.grades.len() >= RECENT_DIARY_WANT {
            return self.finish_recent_diary();
        }

        let (gen, offset) = (job.gen, job.offset);
        let Some(session) = self.session.clone() else {
            self.recent_diary_job = None;
            return RecentDiaryAction::Done(Vec::new());
        };
        let day = chrono::Local::now().date_naive() - chrono::Duration::days(offset);
        let date = day.format("%Y-%m-%d").to_string();
        let label = diary::recent_label(day);
        let cache_gen = self.next_cache_generation();
        RecentDiaryAction::Fetch(RecentDiaryFetch {
            request: diary::request(&session, &date),
            date,
            label,
            gen,
            cache_gen,
        })
    }

    fn finish_recent_diary(&mut self) -> RecentDiaryAction {
        let Some(job) = self.recent_diary_job.take() else {
            return RecentDiaryAction::Ignored;
        };
        self.diary_cache
            .persist(self.store.as_ref(), &self.storage_path, self.cache_format);
        let mut grades = job.grades;
        grades.truncate(diary::RECENT_MAX_ITEMS);
        RecentDiaryAction::Done(grades)
    }

    // ========================================================
    //  Периоды для фоновых задач
    // ========================================================
    pub fn periods(&self) -> &[Period] {
        &self.periods
    }

    pub fn current_period_range(&self) -> Option<(String, String)> {
        marks::current_period_range(&self.periods)
    }

    pub fn load_periods_from_disk(&mut self) -> bool {
        match self.load_periods_disk() {
            Some(raw) => {
                let periods = marks::parse_periods(&raw);
                if periods.is_empty() {
                    return false;
                }
                self.periods = periods;
                true
            }
            None => false,
        }
    }

    /// Фоновый режим: сохранить и разобрать периоды без UI-состояния.
    pub fn absorb_periods(&mut self, raw: &str) -> bool {
        let periods = marks::parse_periods(raw);
        if periods.is_empty() {
            return false;
        }
        self.persist_periods(raw);
        self.periods = periods;
        true
    }

    // ========================================================
    //  Внутренние кеши
    // ========================================================
    fn persist_periods(&self, raw: &str) {
        if let Err(e) = self
            .store
            .save(&self.storage_path, FILE_PERIODS, CONTEXT_PERIODS, raw)
        {
            log::warn!("Не удалось сохранить периоды: {:?}", e);
        }
    }

    fn load_periods_disk(&self) -> Option<String> {
        self.store
            .load(&self.storage_path, FILE_PERIODS, CONTEXT_PERIODS)
            .filter(|raw| marks::parse_periods_checked(raw).is_ok_and(|p| !p.is_empty()))
    }

    fn cache_get(&self, range: &str) -> Option<String> {
        self.grades_cache
            .iter()
            .find(|(k, _)| k == range)
            .map(|(_, v)| v.clone())
    }

    fn cache_put(&mut self, range: &str, raw: &str) {
        if let Some(e) = self.grades_cache.iter_mut().find(|(k, _)| k == range) {
            e.1 = raw.to_string();
        } else {
            self.grades_cache.push((range.to_string(), raw.to_string()));
        }
    }

    fn is_fetched(&self, range: &str) -> bool {
        self.grades_fetched.iter().any(|r| r == range)
    }

    fn mark_fetched(&mut self, range: &str) {
        if !self.is_fetched(range) {
            self.grades_fetched.push(range.to_string());
        }
    }

    fn persist_grades(&self) {
        let json = match serde_json::to_string(&self.grades_cache) {
            Ok(j) => j,
            Err(e) => {
                log::warn!("Не удалось сериализовать кеш оценок: {:?}", e);
                return;
            }
        };
        if let Err(e) =
            self.store
                .save(&self.storage_path, FILE_GRADES_CACHE, CONTEXT_GRADES, &json)
        {
            log::warn!("Не удалось сохранить кеш оценок: {:?}", e);
        }
    }

    fn load_grades_disk(&mut self) {
        let Some(json) = self
            .store
            .load(&self.storage_path, FILE_GRADES_CACHE, CONTEXT_GRADES)
        else {
            return;
        };
        match serde_json::from_str::<Vec<(String, String)>>(&json) {
            Ok(map) => {
                log::info!("Кеш оценок загружен с диска: {} диапазонов", map.len());
                self.grades_cache = map
                    .into_iter()
                    .filter(|(_, raw)| marks::parse_marks_checked(raw).is_ok())
                    .collect();
            }
            Err(e) => log::warn!("Кеш оценок не разобрался: {:?}", e),
        }
    }
}

// ============================================================
//  Вспомогательные функции
// ============================================================
fn date_view(date: &str) -> DateView {
    let (year, month, day) = diary::parse_ymd(date);
    DateView {
        date: date.to_string(),
        year,
        month,
        day,
    }
}

fn error_diary_update(e: &FetchError, date: &str, cache: &DayCache) -> DiaryUpdate {
    DiaryUpdate {
        view: None,
        date: None,
        loading: Some(false),
        error: Some(e.message()),
        stale: Some(diary::stale_hint(cache.fetched_at(date))),
        toggles: None,
    }
}

fn marks_for_subject(raw: &str, subject: &str) -> Vec<Mark> {
    marks::parse_marks(raw)
        .into_iter()
        .find(|s| s.subject == subject)
        .map(|s| s.marks)
        .unwrap_or_default()
}

fn quarter_request(session: &UserSession, period: &Period) -> ApiRequest {
    marks::marks_request(
        session,
        &marks::to_iso(&period.from),
        &marks::to_iso(&period.to),
    )
}

fn demo_session() -> UserSession {
    UserSession {
        sid: "demo".into(),
        user_guid: "demo_student".into(),
        apikey: "demo".into(),
        full_name: "Иван Петров".into(),
        school_name: "МБОУ СОШ №1 г. Орёл".into(),
        school_class: "9 «А»".into(),
        is_parent: false,
        parent_name: String::new(),
        children: Vec::new(),
    }
}

// Для демо-переключения ребёнка адаптер может собрать сессию сам.
pub fn demo_child(guid: &str) -> Option<ChildInfo> {
    Some(ChildInfo {
        guid: guid.to_string(),
        full_name: "Иван Петров".into(),
        school_name: "МБОУ СОШ №1 г. Орёл".into(),
        school_class: "9 «А»".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::HttpResponse;
    use crate::crypto::{CONTEXT_EVENTS, CONTEXT_MARKS_CACHE, FILE_EVENTS};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // In-memory SecureStore: ядру важна семантика файлов, а конверт
    // проверяется отдельно в crypto.rs.
    #[derive(Clone, Default)]
    struct MemStore {
        files: Arc<Mutex<HashMap<String, String>>>,
    }

    impl SecureStore for MemStore {
        fn load(&self, _path: &str, filename: &str, _ctx: &[u8]) -> Option<String> {
            self.files.lock().unwrap().get(filename).cloned()
        }
        fn save(&self, _path: &str, filename: &str, _ctx: &[u8], data: &str) -> Result<(), String> {
            self.files
                .lock()
                .unwrap()
                .insert(filename.to_string(), data.to_string());
            Ok(())
        }
        fn delete(&self, _path: &str, filename: &str) {
            self.files.lock().unwrap().remove(filename);
        }
    }

    const DIARY: &str = r#"{"success":true,"data":[
        {"LESSON_NUMBER":1,"SUBJECT_NAME":"Алгебра","HOMEWORK":"№1","MARKS":[{"SHORT_NAME":"5","VALUE":5}]}
    ]}"#;
    const PERIODS: &str = r#"{"success":true,"data":[
        {"NAME":"1 четверть","BEGIN_DATE":"01.09.2025","DATE_TO":"27.10.2025"}
    ]}"#;
    const MARKS: &str = r#"{"success":true,"data":[{"SUBJECT_NAME":"Алгебра","MARKS":[
        {"SHORT_NAME":"5","VALUE":5,"DATE":"01.10.2025"}
    ]}]}"#;

    fn ok(body: &str) -> HttpResult {
        Ok(HttpResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        })
    }

    fn offline() -> HttpResult {
        Err(crate::api::TransportError::Offline)
    }

    fn core(store: &MemStore) -> Core {
        let mut core = Core::new("/data", Box::new(store.clone()), CacheFormat::Stamps);
        core.adopt_session(demo_session(), false).unwrap();
        core.set_current_date("2026-02-02".into());
        core
    }

    #[test]
    fn demo_subject_filter_and_notes_survive_reapply_and_reload() {
        let store = MemStore::default();
        let mut core = core(&store);
        core.enter_demo();
        core.set_current_date("2026-02-02".into());
        let initial = core.refresh_diary(0).unwrap();
        let DiaryAction::Render(initial) = initial else {
            panic!("demo must render without network")
        };
        let lessons = initial.view.unwrap().lessons;
        assert!(!lessons.is_empty());
        let subject = lessons[0].subject.clone();
        let key = lessons[0].note_key.clone();
        core.save_note(&key, "Не забыть тетрадь");
        let hidden = core.set_subject_hidden(&subject, true);
        assert!(hidden
            .toggles
            .iter()
            .any(|t| t.subject == subject && t.hidden));
        let filtered = hidden.diary.unwrap().view.unwrap().lessons;
        assert!(filtered.iter().all(|l| l.subject != subject));
        assert_eq!(
            filtered.len(),
            lessons.iter().filter(|l| l.subject != subject).count()
        );
        let mut restored = Core::new("/data", Box::new(store.clone()), CacheFormat::Stamps);
        restored.enter_demo();
        restored.set_current_date("2026-02-02".into());
        assert!(restored
            .subject_toggles()
            .iter()
            .any(|t| t.subject == subject && t.hidden));
        let shown = restored
            .set_subject_hidden(&subject, false)
            .diary
            .unwrap()
            .view
            .unwrap()
            .lessons;
        assert_eq!(shown.len(), lessons.len());
        assert_eq!(
            shown.iter().find(|l| l.note_key == key).unwrap().note,
            "Не забыть тетрадь"
        );
        restored.save_note(&key, "  ");
        let cleared = restored.reapply_current_day().unwrap().view.unwrap();
        assert_eq!(
            cleared
                .lessons
                .iter()
                .find(|l| l.note_key == key)
                .unwrap()
                .note,
            ""
        );
    }

    #[test]
    fn diary_fetches_once_then_serves_cache() {
        let store = MemStore::default();
        let mut core = core(&store);

        let fetch = match core.refresh_diary(0).unwrap() {
            DiaryAction::Fetch { fetch, .. } => fetch,
            DiaryAction::Render(_) => panic!("ожидался запрос"),
        };
        let update = core.complete_diary(&fetch, ok(DIARY)).unwrap();
        let view = update.view.unwrap();
        assert_eq!(view.lessons.len(), 1);
        assert_eq!(view.lessons[0].subject, "Алгебра");
        assert_eq!(view.lessons[0].mark, "5");
        assert!(store.files.lock().unwrap().contains_key(FILE_MARKS_CACHE));

        // Второй refresh отдаёт кеш без запроса.
        assert!(matches!(
            core.refresh_diary(0).unwrap(),
            DiaryAction::Render(_)
        ));
    }

    #[test]
    fn stale_diary_response_is_dropped() {
        let store = MemStore::default();
        let mut core = core(&store);

        let first = match core.refresh_diary(0).unwrap() {
            DiaryAction::Fetch { fetch, .. } => fetch,
            DiaryAction::Render(_) => panic!("ожидался запрос"),
        };
        // Пока ответ в пути, пользователь дёрнул день ещё раз.
        assert!(matches!(
            core.refresh_diary(0).unwrap(),
            DiaryAction::Fetch { .. }
        ));
        assert!(core.complete_diary(&first, ok(DIARY)).is_none());
    }

    #[test]
    fn marks_flow_periods_marks_and_cache() {
        let store = MemStore::default();
        let mut core = core(&store);

        let gen = match core.init_marks().unwrap() {
            MarksAction::FetchPeriods { gen, .. } => gen,
            _ => panic!("ожидался запрос периодов"),
        };
        let fetch = match core.complete_periods(gen, ok(PERIODS)).unwrap() {
            MarksAction::FetchMarks { fetch, .. } => fetch,
            _ => panic!("ожидался запрос оценок"),
        };
        let update = core.complete_marks(&fetch, ok(MARKS)).unwrap();
        assert_eq!(update.subjects.unwrap()[0].subject, "Алгебра");

        assert!(matches!(
            core.select_period(0).unwrap(),
            MarksAction::Render(_)
        ));
    }

    #[test]
    fn periods_fall_back_to_disk_when_offline() {
        let store = MemStore::default();
        {
            let mut core = core(&store);
            let gen = match core.init_marks().unwrap() {
                MarksAction::FetchPeriods { gen, .. } => gen,
                _ => panic!(),
            };
            core.complete_periods(gen, ok(PERIODS));
        }

        let mut restarted = core(&store);
        let gen = match restarted.init_marks().unwrap() {
            MarksAction::FetchPeriods { gen, .. } => gen,
            _ => panic!(),
        };
        assert!(matches!(
            restarted.complete_periods(gen, offline()).unwrap(),
            MarksAction::FetchMarks { .. }
        ));
    }

    #[test]
    fn switching_child_keeps_session_but_clears_caches() {
        let store = MemStore::default();
        let mut core = core(&store);
        core.save_session(&demo_session()).unwrap();
        core.store()
            .save("/data", FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE, "{}")
            .unwrap();

        core.apply_switched_session(demo_session()).unwrap();

        let files = store.files.lock().unwrap();
        assert!(
            files.contains_key(FILE_SESSION),
            "сессия должна пережить переключение"
        );
        assert!(
            !files.contains_key(FILE_MARKS_CACHE),
            "кеши должны сброситься"
        );
    }

    #[test]
    fn logout_removes_session_and_server_caches_but_keeps_local_data() {
        let store = MemStore::default();
        let mut core = core(&store);
        core.save_session(&demo_session()).unwrap();
        core.store()
            .save("/data", FILE_MARKS_CACHE, CONTEXT_MARKS_CACHE, "{}")
            .unwrap();
        core.store()
            .save("/data", FILE_EVENTS, CONTEXT_EVENTS, "{}")
            .unwrap();

        core.logout();

        let files = store.files.lock().unwrap();
        assert!(!files.contains_key(FILE_SESSION));
        assert!(!files.contains_key(FILE_MARKS_CACHE));
        assert!(files.contains_key(FILE_EVENTS), "свои события не удаляем");
        assert!(core.session().is_none());
    }
}
