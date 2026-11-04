// Домен и бизнес-логика без Slint, reqwest и tokio.
//
// Ядро не выполняет HTTP и не хранит ключи: оно готовит `ApiRequest`,
// адаптер платформы выполняет запрос и возвращает `HttpResult` обратно.

pub mod api;
pub mod cache;
pub mod core;
pub mod crypto;
pub mod diary;
pub mod finals;
pub mod login;
pub mod marks;
pub mod model;
pub mod notify;
pub mod ports;
pub mod stores;
pub mod update;

pub use api::{ApiRequest, FetchError, HttpResponse, HttpResult, TransportError};
pub use cache::DayCache;
pub use core::{
    ChartFetch, ChartStep, Core, DiaryAction, DiaryFetch, DiaryUpdate, FinalsAction, FinalsFetch,
    FinalsUpdate, MarksAction, MarksFetch, MarksUpdate, RecentAction, RecentDiaryAction,
    RecentDiaryFetch, RecentFetch, SubjectFilterUpdate,
};
pub use crypto::{
    ahh_encrypt, decrypt_blob, encrypt_blob, random_bytes, CONTEXT_EVENTS, CONTEXT_FINALS,
    CONTEXT_GRADES, CONTEXT_HOMEWORK, CONTEXT_MARKS_CACHE, CONTEXT_NOTES, CONTEXT_NOTIFY,
    CONTEXT_PERIODS, CONTEXT_SESSION, CONTEXT_SUBJECTS, FILE_EVENTS, FILE_FINALS_CACHE,
    FILE_GRADES_CACHE, FILE_GRADES_NOTIFY, FILE_HOMEWORK, FILE_MARKS_CACHE, FILE_NOTES,
    FILE_PERIODS, FILE_SESSION, FILE_SUBJECTS,
};
pub use diary::DiaryLessonData;
pub use login::{
    build_parent_session, build_student_session, init_session_request, login_request,
    parse_init_http, parse_login_http, LoginAccount, LoginError,
};
pub use model::*;
pub use ports::{CacheFormat, SecureStore};
pub use stores::{EventsStore, HomeworkStore, NotesStore, SubjectsStore};
pub use update::UpdateInfo;
