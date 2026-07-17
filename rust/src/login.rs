// Авторизация через X1_SSO-токен и сборка UserSession.
use serde::{Deserialize, Serialize};

use crate::crypto::{self, UserSession};
use crate::net::http_client;

// Таймаут на сетевые запросы логина (как у дневника — против зависания на VPN)
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// Типизированные ошибки входа — для понятного сообщения пользователю
pub(crate) enum LoginError {
    Offline,                // нет соединения
    Timeout,                // сервер не отвечает (VPN / иностранный IP)
    ServerRejected(String), // сервер ответил, но отказал / прислал мусор
    BadData,                // в ответе нет данных ученика (PARTICIPANT)
    Storage,                // не удалось сохранить сессию (Keystore/диск)
}

impl LoginError {
    pub(crate) fn user_message(&self) -> String {
        match self {
            LoginError::Offline => "Нет соединения с интернетом. Проверьте сеть.".into(),
            LoginError::Timeout => "Сервер не отвечает. Отключите VPN — доступ только из РФ.".into(),
            LoginError::ServerRejected(m) if m.is_empty() =>
                "Сервер отклонил вход. Попробуйте ещё раз.".into(),
            LoginError::ServerRejected(m) => format!("Сервер отклонил вход: {m}"),
            LoginError::BadData => "Не удалось получить данные ученика. Попробуйте войти ещё раз.".into(),
            LoginError::Storage => "Не удалось сохранить вход на устройстве.".into(),
        }
    }
}

// reqwest-ошибка → тип входа (таймаут отличаем от прочего)
fn net_err(e: reqwest::Error) -> LoginError {
    if e.is_timeout() {
        LoginError::Timeout
    } else {
        LoginError::Offline
    }
}

#[derive(Serialize)]
struct LoginPayload {
    sid: String,
    api_key: String,
}
#[derive(Deserialize)]
struct LoginResponse {
    success: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<LoginData>,
}

#[derive(Serialize)]
struct InitSessionPayload {
    sid: String,
    apikey: String,
    sysguid: String,
}
#[derive(Deserialize)]
struct InitSessionResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
struct LoginData {
    #[serde(rename = "SCHOOLS", default)]
    schools: Vec<SchoolEntry>,
}
#[derive(Deserialize)]
struct SchoolEntry {
    #[serde(rename = "SCHOOL")]
    school: SchoolInfo,
    #[serde(rename = "PARTICIPANT", default)]
    participant: Option<Participant>,
}
#[derive(Deserialize)]
struct SchoolInfo {
    #[serde(rename = "NAME", default)]
    name: String,
    #[serde(rename = "SHORT_NAME", default)]
    short_name: String,
}
#[derive(Deserialize)]
struct Participant {
    #[serde(rename = "SYS_GUID")]
    sys_guid: String,
    #[serde(rename = "SURNAME", default)]
    surname: String,
    #[serde(rename = "NAME", default)]
    name: String,
    #[serde(rename = "SECONDNAME", default)]
    secondname: String,
    #[serde(rename = "GRADE", default)]
    grade: Option<Grade>,
}
#[derive(Deserialize)]
struct Grade {
    #[serde(rename = "NAME", default)]
    name: String,
}

pub(crate) async fn login_and_save(
    sid: &str,
    storage_path: &str,
) -> Result<UserSession, LoginError> {
    let cleaned_sid = sid.trim();

    // 1. Тянем данные ученика
    let resp = login_with_cookie(cleaned_sid).await?;
    if !resp.success {
        return Err(LoginError::ServerRejected(resp.message));
    }
    
    // 2. Регистрируем сессию
    let data = resp.data.as_ref().ok_or(LoginError::BadData)?;
    let entry = data.schools.iter()
        .find(|s| s.participant.is_some())
        .ok_or(LoginError::BadData)?;
    let p = entry.participant.as_ref().ok_or(LoginError::BadData)?;
    let guid = p.sys_guid.trim();
    
    if guid.is_empty() {
        return Err(LoginError::BadData);
    }
    
    let key = init_session(cleaned_sid, &guid).await?;
    
    // 3. Сборка сессии
    let session = build_session(cleaned_sid, &key, &resp).ok_or(LoginError::BadData)?;

    crypto::save_session(storage_path, &session).map_err(|e| {
        log::error!("Ошибка сохранения сессии: {:?}", e);
        LoginError::Storage
    })?;
    
    Ok(session)
}

async fn login_with_cookie(sid: &str) -> Result<LoginResponse, LoginError> {
    let url = "https://mp2.obr57.ru/journals/login";
    let api_key = crypto::ahh_encrypt(sid);
    let payload = LoginPayload { sid: sid.to_string(), api_key };
    let body = http_client()
        .post(url)
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 13)")
        .header("Content-Type", "application/json")
        .header("X-Requested-With", "ru.integrics.orelschool")
        .timeout(LOGIN_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(net_err)?
        .text()
        .await
        .map_err(net_err)?;
    serde_json::from_str(&body).map_err(|e| {
        log::error!("Логин: не разобрать ответ: {:?}", e);
        LoginError::ServerRejected(String::new())
    })
}

async fn init_session(x1_sso_cookie: &str, guid: &str) -> Result<String, LoginError> {
    let url = "https://mp2.obr57.ru/session/initsession";
    let payload = InitSessionPayload {
        sid: crypto::ahh_encrypt(x1_sso_cookie),
        // фиксированный app-apikey из реверса (для initsession, не сессионный)
        apikey: "0xt25240s9s12xv767v1ll17757e32e34x12ppix332vdi2i".to_string(),
        sysguid: crypto::ahh_encrypt(guid),
    };

    let body = http_client()
        .post(url)
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 13)")
        .header("Content-Type", "application/json")
        .header("X-Requested-With", "ru.integrics.orelschool")
        .timeout(LOGIN_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(net_err)?
        .text()
        .await
        .map_err(net_err)?;

    let resp: InitSessionResponse = serde_json::from_str(&body).map_err(|e| {
        log::error!("initsession: не разобрать ответ: {:?}", e);
        LoginError::ServerRejected(String::new())
    })?;
    if resp.key.is_empty() {
        return Err(LoginError::ServerRejected(format!(
            "пустой ключ (status={})",
            resp.status
        )));
    }
    Ok(resp.key)
}

fn build_session(sid: &str, apikey: &str, resp: &LoginResponse) -> Option<UserSession> {
    let data = resp.data.as_ref()?;
    let entry = data.schools.iter().find(|s| s.participant.is_some())?;
    let p = entry.participant.as_ref()?;
    let full_name = [p.surname.trim(), p.name.trim(), p.secondname.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let school_name = if !entry.school.name.trim().is_empty() {
        entry.school.name.trim().to_string()
    } else {
        entry.school.short_name.trim().to_string()
    };
    let school_class = p.grade.as_ref().map(|g| g.name.trim().to_string()).unwrap_or_default();
    Some(UserSession {
        sid: sid.to_string(),
        user_guid: p.sys_guid.clone(),
        full_name,
        school_name,
        school_class,
        apikey: apikey.to_string(),
    })
}
