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
#[derive(Deserialize, Debug)]
pub(crate) struct LoginResponse {
    pub success: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Option<LoginData>,
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

#[derive(Deserialize, Debug)]
pub(crate) struct LoginData {
    #[serde(rename = "LOGIN", default)]
    pub login: String,
    #[serde(rename = "SURNAME", default)]
    pub surname: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SECONDNAME", default)]
    pub secondname: String,
    #[serde(rename = "EMAIL", default)]
    pub email: String,
    #[serde(rename = "SCHOOLS", default)]
    pub schools: Vec<SchoolEntry>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct SchoolEntry {
    #[serde(rename = "ROLES", default)]
    pub roles: Vec<String>,
    #[serde(rename = "SCHOOL", default)]
    pub school: Option<SchoolInfo>,
    #[serde(rename = "PARENT", default)]
    pub parent: Option<ParentInfo>,
    #[serde(rename = "PARTICIPANT", default)]
    pub participant: Option<Participant>,
    #[serde(rename = "USER_PARTICIPANTS", default)]
    pub user_participants: Vec<UserParticipant>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct SchoolInfo {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "ID", default)]
    pub id: Option<i64>,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SHORT_NAME", default)]
    pub short_name: String,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct ParentInfo {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "SURNAME", default)]
    pub surname: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SECONDNAME", default)]
    pub secondname: String,
    #[serde(rename = "SCHOOL", default)]
    pub school: Option<SchoolInfo>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct Participant {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "SURNAME", default)]
    pub surname: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SECONDNAME", default)]
    pub secondname: String,
    #[serde(rename = "GRADE", default)]
    pub grade: Option<Grade>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct UserParticipant {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "SURNAME", default)]
    pub surname: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SECONDNAME", default)]
    pub secondname: String,
    #[serde(rename = "GRADE", default)]
    pub grade: Option<Grade>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct Grade {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SCHOOL", default)]
    pub school: Option<SchoolInfo>,
}

#[derive(Debug, Clone)]
pub(crate) enum LoginOutcome {
    Success(UserSession),
    NeedChildSelection {
        sid: String,
        storage_path: String,
        children: Vec<crypto::ChildInfo>,
        parent_name: String,
    },
}

pub(crate) fn is_parent_account(resp: &LoginResponse) -> bool {
    if let Some(data) = &resp.data {
        for s in &data.schools {
            if s.roles.iter().any(|r| r.eq_ignore_ascii_case("parents") || r.eq_ignore_ascii_case("parent")) {
                return true;
            }
            if !s.user_participants.is_empty() {
                return true;
            }
            if s.parent.is_some() {
                return true;
            }
        }
    }
    false
}

pub(crate) fn extract_parent_name(resp: &LoginResponse) -> String {
    if let Some(data) = &resp.data {
        for s in &data.schools {
            if let Some(p) = &s.parent {
                let name = [p.surname.trim(), p.name.trim(), p.secondname.trim()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                if !name.is_empty() {
                    return name;
                }
            }
        }
        let direct_name = [data.surname.trim(), data.name.trim(), data.secondname.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if !direct_name.is_empty() {
            return direct_name;
        }
    }
    String::new()
}

pub(crate) fn extract_children(resp: &LoginResponse) -> Vec<crypto::ChildInfo> {
    let mut children = Vec::new();
    if let Some(data) = &resp.data {
        for school_entry in &data.schools {
            let default_school_name = school_entry.school.as_ref().map(|s| {
                if !s.name.trim().is_empty() {
                    s.name.trim().to_string()
                } else {
                    s.short_name.trim().to_string()
                }
            }).unwrap_or_default();

            for p in &school_entry.user_participants {
                let guid = p.sys_guid.trim().to_string();
                if guid.is_empty() {
                    continue;
                }
                let full_name = [p.surname.trim(), p.name.trim(), p.secondname.trim()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let full_name = if full_name.is_empty() {
                    "Ученик".to_string()
                } else {
                    full_name
                };

                let school_class = p.grade.as_ref()
                    .map(|g| g.name.trim().to_string())
                    .unwrap_or_default();

                let school_name = p.grade.as_ref()
                    .and_then(|g| g.school.as_ref())
                    .map(|s| {
                        if !s.name.trim().is_empty() {
                            s.name.trim().to_string()
                        } else {
                            s.short_name.trim().to_string()
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| default_school_name.clone());

                if !children.iter().any(|c: &crypto::ChildInfo| c.guid == guid) {
                    children.push(crypto::ChildInfo {
                        guid,
                        full_name,
                        school_name,
                        school_class,
                    });
                }
            }
        }
    }
    children
}

pub(crate) fn build_parent_session(
    sid: &str,
    apikey: &str,
    child: &crypto::ChildInfo,
    parent_name: String,
    children: Vec<crypto::ChildInfo>,
) -> UserSession {
    UserSession {
        sid: sid.to_string(),
        user_guid: child.guid.clone(),
        apikey: apikey.to_string(),
        full_name: child.full_name.clone(),
        school_name: child.school_name.clone(),
        school_class: child.school_class.clone(),
        is_parent: true,
        parent_name,
        children,
    }
}

pub(crate) fn build_student_session(
    sid: &str,
    apikey: &str,
    resp: &LoginResponse,
) -> Option<UserSession> {
    let data = resp.data.as_ref()?;
    let entry = data.schools.iter().find(|s| s.participant.is_some())?;
    let p = entry.participant.as_ref()?;
    let full_name = [p.surname.trim(), p.name.trim(), p.secondname.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let school_name = entry.school.as_ref().map(|s| {
        if !s.name.trim().is_empty() {
            s.name.trim().to_string()
        } else {
            s.short_name.trim().to_string()
        }
    }).unwrap_or_default();
    let school_class = p.grade.as_ref().map(|g| g.name.trim().to_string()).unwrap_or_default();
    Some(UserSession {
        sid: sid.to_string(),
        user_guid: p.sys_guid.clone(),
        full_name,
        school_name,
        school_class,
        apikey: apikey.to_string(),
        is_parent: false,
        parent_name: String::new(),
        children: Vec::new(),
    })
}

pub(crate) async fn login_and_save(
    sid: &str,
    storage_path: &str,
) -> Result<LoginOutcome, LoginError> {
    let cleaned_sid = sid.trim();

    // 1. Тянем данные логина
    let resp = login_with_cookie(cleaned_sid).await?;
    if !resp.success {
        return Err(LoginError::ServerRejected(resp.message));
    }

    if is_parent_account(&resp) {
        let children = extract_children(&resp);
        if children.is_empty() {
            log::error!("Родительский аккаунт, но список USER_PARTICIPANTS пуст");
            return Err(LoginError::BadData);
        }
        let parent_name = extract_parent_name(&resp);

        if children.len() == 1 {
            // 1 ребёнок — подключаем сразу
            let child = &children.clone()[0];
            let key = init_session(cleaned_sid, &child.guid).await?;
            let session = build_parent_session(cleaned_sid, &key, child, parent_name, children);
            crypto::save_session(storage_path, &session).map_err(|e| {
                log::error!("Ошибка сохранения сессии родителя: {:?}", e);
                LoginError::Storage
            })?;
            Ok(LoginOutcome::Success(session))
        } else {
            // > 1 детей — показываем выбор ребёнка
            Ok(LoginOutcome::NeedChildSelection {
                sid: cleaned_sid.to_string(),
                storage_path: storage_path.to_string(),
                children,
                parent_name,
            })
        }
    } else {
        // Ученик
        let data = resp.data.as_ref().ok_or(LoginError::BadData)?;
        let entry = data.schools.iter()
            .find(|s| s.participant.is_some())
            .ok_or(LoginError::BadData)?;
        let p = entry.participant.as_ref().ok_or(LoginError::BadData)?;
        let guid = p.sys_guid.trim();
        if guid.is_empty() {
            return Err(LoginError::BadData);
        }
        let key = init_session(cleaned_sid, guid).await?;
        let session = build_student_session(cleaned_sid, &key, &resp).ok_or(LoginError::BadData)?;

        crypto::save_session(storage_path, &session).map_err(|e| {
            log::error!("Ошибка сохранения сессии ученика: {:?}", e);
            LoginError::Storage
        })?;
        Ok(LoginOutcome::Success(session))
    }
}
pub(crate) async fn complete_child_login(
    sid: &str,
    storage_path: &str,
    child_guid: &str,
    parent_name: String,
    children: Vec<crypto::ChildInfo>,
) -> Result<UserSession, LoginError> {
    let child = children.iter().find(|c| c.guid == child_guid).cloned().ok_or(LoginError::BadData)?;
    let key = init_session(sid, child_guid).await?;
    let session = build_parent_session(sid, &key, &child, parent_name, children);
    crypto::save_session(storage_path, &session).map_err(|e| {
        log::error!("Ошибка сохранения сессии выбранного ребёнка: {:?}", e);
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

pub(crate) async fn init_session(x1_sso_cookie: &str, guid: &str) -> Result<String, LoginError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_parent_login() {
        let json_data = r#"{"success":true,"system":false,"message":"ok","data":{"LOGIN":"","SURNAME":"","NAME":"ЮЛИЯ","SECONDNAME":"","EMAIL":"mail.ru","CONFIRMATION":"NONE","CONFIRM_EXPIRATION":0,"SESSION_ID":null,"SCHOOLS":[{"ROLES":["parents"],"SCHOOL":{"SYS_GUID":"20265737743","ID":37743,"NAME":"Муниципальное бюджетное общеобразовательное учреждение - средняя общеобразовательная школа ","SHORT_NAME":"Муниципальная бюджетная средняя общеобразовательная школа "},"GOVERNMENT":null,"TEACHER":null,"PARENT":{"SYS_GUID":"5DC8AC2866794A2C19A762D39ED902AC","SURNAME":"","NAME":"Юлия","SECONDNAME":"","SCHOOL":{"SYS_GUID":"20265737743","ID":37743,"NAME":"Муниципальное бюджетное общеобразовательное учреждение - средняя общеобразовательная школа ","SHORT_NAME":"Муниципальная бюджетная средняя общеобразовательная школа № "}},"PARTICIPANT":null,"USER_GRADES":[],"USER_PARTICIPANTS":[{"SYS_GUID":"E0D7B0D9E03833121953FA175C94F3DA","SURNAME":"","NAME":"Андрей","SECONDNAME":"","GRADE":{"SYS_GUID":"F2D25C2A3DE8E9C844F96C201A07F920","NAME":"9Г","SCHOOL":{"SYS_GUID":"20265737743","ID":37743,"NAME":"Муниципальное бюджетное общеобразовательное учреждение - средняя общеобразовательная ","SHORT_NAME":"Муниципальная бюджетная средняя общеобразовательная школа "},"GRADE_HEAD":{"SYS_GUID":"EF3B9C815CCC4B35D8A99B73259A99E0","SURNAME":"Б","NAME":"","SECONDNAME":"","SCHOOL":{"SYS_GUID":"20265737743","ID":37743,"NAME":"Муниципальное бюджетное общеобразовательное учреждение - ","SHORT_NAME":"Муниципальная бюджетная средняя общеобразовательная школа "}}}}],"USER_PARENTS":[]}]}}"#;

        let resp: LoginResponse = serde_json::from_str(json_data).expect("failed to deserialize");
        assert!(resp.success);
        assert!(is_parent_account(&resp));
        let children = extract_children(&resp);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].guid, "E0D7B0D9E03833121953FA175C94F3DA");
        assert_eq!(children[0].full_name, "Андрей");
        assert_eq!(children[0].school_class, "9Г");
        assert_eq!(extract_parent_name(&resp), "Юлия");
    }
}
