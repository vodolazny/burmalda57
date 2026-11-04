// Вход через X1_SSO: DTO ответа журнала, разбор аккаунта и сборка сессии.
//
// Сетевые шаги остаются адаптерам: core отдаёт готовые ApiRequest и умеет
// разобрать ответ. Одинаково используется мобильным слоем и вебом.

use serde::{Deserialize, Serialize};

use crate::api::{ApiRequest, HttpResponse, TransportError};
use crate::crypto::ahh_encrypt;
use crate::model::{ChildInfo, UserSession};

const LOGIN_TIMEOUT_SECS: u64 = 10;
// Фиксированный app-apikey из реверса (для initsession, не сессионный).
const INIT_APP_APIKEY: &str = "0xt25240s9s12xv767v1ll17757e32e34x12ppix332vdi2i";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    Offline,
    Timeout,
    ServerRejected(String),
    BadData,
    Storage,
}

impl LoginError {
    pub fn user_message(&self) -> String {
        match self {
            LoginError::Offline => "Нет соединения с интернетом. Проверьте сеть.".into(),
            LoginError::Timeout => {
                "Сервер не отвечает. Отключите VPN — доступ только из РФ.".into()
            }
            LoginError::ServerRejected(m) if m.is_empty() => {
                "Сервер отклонил вход. Попробуйте ещё раз.".into()
            }
            LoginError::ServerRejected(m) => format!("Сервер отклонил вход: {m}"),
            LoginError::BadData => {
                "Не удалось получить данные ученика. Попробуйте войти ещё раз.".into()
            }
            LoginError::Storage => "Не удалось сохранить вход на устройстве.".into(),
        }
    }

    pub fn from_transport(e: TransportError) -> Self {
        match e {
            TransportError::Timeout => LoginError::Timeout,
            TransportError::Offline => LoginError::Offline,
        }
    }
}

// ============================================================
//  DTO
// ============================================================
#[derive(Serialize)]
struct LoginPayload {
    sid: String,
    api_key: String,
}

#[derive(Serialize)]
struct InitSessionPayload {
    sid: String,
    apikey: String,
    sysguid: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoginResponse {
    pub success: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Option<LoginData>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoginData {
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

#[derive(Deserialize, Debug, Clone)]
pub struct SchoolEntry {
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
pub struct SchoolInfo {
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
pub struct ParentInfo {
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
pub struct Participant {
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
pub struct UserParticipant {
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
pub struct Grade {
    #[serde(rename = "SYS_GUID", default)]
    pub sys_guid: String,
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "SCHOOL", default)]
    pub school: Option<SchoolInfo>,
}

#[derive(Deserialize)]
struct InitSessionResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    key: String,
}

#[derive(Clone, Debug)]
pub enum LoginAccount {
    Student {
        guid: String,
    },
    Parent {
        children: Vec<ChildInfo>,
        parent_name: String,
    },
}

// ============================================================
//  Запросы
// ============================================================
pub fn login_request(sid: &str) -> ApiRequest {
    let payload = LoginPayload {
        sid: sid.to_string(),
        api_key: ahh_encrypt(sid),
    };
    ApiRequest::post_json(
        &format!("{}/journals/login", crate::api::API_BASE),
        &payload,
        LOGIN_TIMEOUT_SECS,
    )
}

pub fn init_session_request(sid: &str, guid: &str) -> ApiRequest {
    let payload = InitSessionPayload {
        sid: ahh_encrypt(sid),
        apikey: INIT_APP_APIKEY.to_string(),
        sysguid: ahh_encrypt(guid),
    };
    ApiRequest::post_json(
        &format!("{}/session/initsession", crate::api::API_BASE),
        &payload,
        LOGIN_TIMEOUT_SECS,
    )
}

// ============================================================
//  Разбор ответов
// ============================================================
pub fn parse_login_http(resp: HttpResponse) -> Result<LoginResponse, LoginError> {
    let body = String::from_utf8_lossy(&resp.body).to_string();
    let parsed = serde_json::from_str::<LoginResponse>(&body);
    match parsed {
        Ok(r) => Ok(r),
        Err(e) => {
            log::error!("Логин: не разобрать ответ (HTTP {}): {:?}", resp.status, e);
            Err(LoginError::ServerRejected(String::new()))
        }
    }
}

pub fn parse_init_http(resp: HttpResponse) -> Result<String, LoginError> {
    let body = String::from_utf8_lossy(&resp.body).to_string();
    let parsed: InitSessionResponse = serde_json::from_str(&body).map_err(|e| {
        log::error!(
            "initsession: не разобрать ответ (HTTP {}): {:?}",
            resp.status,
            e
        );
        LoginError::ServerRejected(String::new())
    })?;
    if parsed.key.is_empty() {
        return Err(LoginError::ServerRejected(format!(
            "пустой ключ (status={})",
            parsed.status
        )));
    }
    Ok(parsed.key)
}

pub fn account_from(resp: &LoginResponse) -> Result<LoginAccount, LoginError> {
    if !resp.success {
        return Err(LoginError::ServerRejected(resp.message.clone()));
    }
    if is_parent_account(resp) {
        let children = extract_children(resp);
        if children.is_empty() {
            log::error!("Родительский аккаунт, но список USER_PARTICIPANTS пуст");
            return Err(LoginError::BadData);
        }
        return Ok(LoginAccount::Parent {
            children,
            parent_name: extract_parent_name(resp),
        });
    }
    let data = resp.data.as_ref().ok_or(LoginError::BadData)?;
    let entry = data
        .schools
        .iter()
        .find(|s| s.participant.is_some())
        .ok_or(LoginError::BadData)?;
    let p = entry.participant.as_ref().ok_or(LoginError::BadData)?;
    let guid = p.sys_guid.trim();
    if guid.is_empty() {
        return Err(LoginError::BadData);
    }
    Ok(LoginAccount::Student {
        guid: guid.to_string(),
    })
}

pub fn is_parent_account(resp: &LoginResponse) -> bool {
    if let Some(data) = &resp.data {
        for s in &data.schools {
            if s.roles
                .iter()
                .any(|r| r.eq_ignore_ascii_case("parents") || r.eq_ignore_ascii_case("parent"))
            {
                return true;
            }
            if !s.user_participants.is_empty() || s.parent.is_some() {
                return true;
            }
        }
    }
    false
}

fn join_name(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn extract_parent_name(resp: &LoginResponse) -> String {
    if let Some(data) = &resp.data {
        for s in &data.schools {
            if let Some(p) = &s.parent {
                let name = join_name(&[&p.surname, &p.name, &p.secondname]);
                if !name.is_empty() {
                    return name;
                }
            }
        }
        return join_name(&[&data.surname, &data.name, &data.secondname]);
    }
    String::new()
}

fn school_name(s: &SchoolInfo) -> String {
    if !s.name.trim().is_empty() {
        s.name.trim().to_string()
    } else {
        s.short_name.trim().to_string()
    }
}

pub fn extract_children(resp: &LoginResponse) -> Vec<ChildInfo> {
    let mut children = Vec::new();
    if let Some(data) = &resp.data {
        for entry in &data.schools {
            let default_school = entry.school.as_ref().map(school_name).unwrap_or_default();
            for p in &entry.user_participants {
                let guid = p.sys_guid.trim().to_string();
                if guid.is_empty() {
                    continue;
                }
                let full_name = join_name(&[&p.surname, &p.name, &p.secondname]);
                let full_name = if full_name.is_empty() {
                    "Ученик".to_string()
                } else {
                    full_name
                };
                let school_class = p
                    .grade
                    .as_ref()
                    .map(|g| g.name.trim().to_string())
                    .unwrap_or_default();
                let school = p
                    .grade
                    .as_ref()
                    .and_then(|g| g.school.as_ref())
                    .map(school_name)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| default_school.clone());
                if !children.iter().any(|c: &ChildInfo| c.guid == guid) {
                    children.push(ChildInfo {
                        guid,
                        full_name,
                        school_name: school,
                        school_class,
                    });
                }
            }
        }
    }
    children
}

pub fn build_student_session(sid: &str, apikey: &str, resp: &LoginResponse) -> Option<UserSession> {
    let data = resp.data.as_ref()?;
    let entry = data.schools.iter().find(|s| s.participant.is_some())?;
    let p = entry.participant.as_ref()?;
    let full_name = join_name(&[&p.surname, &p.name, &p.secondname]);
    let school = entry.school.as_ref().map(school_name).unwrap_or_default();
    let school_class = p
        .grade
        .as_ref()
        .map(|g| g.name.trim().to_string())
        .unwrap_or_default();
    Some(UserSession {
        sid: sid.to_string(),
        user_guid: p.sys_guid.trim().to_string(),
        full_name,
        school_name: school,
        school_class,
        apikey: apikey.to_string(),
        is_parent: false,
        parent_name: String::new(),
        children: Vec::new(),
    })
}

pub fn build_parent_session(
    sid: &str,
    apikey: &str,
    child: &ChildInfo,
    parent_name: String,
    children: Vec<ChildInfo>,
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

#[cfg(test)]
mod tests {
    use super::*;

    const STUDENT: &str = r#"{
      "success": true,
      "data": {
        "SURNAME": "Петров", "NAME": "Иван", "SECONDNAME": "Сергеевич",
        "SCHOOLS": [{
          "ROLES": ["student"],
          "SCHOOL": {"NAME": "Школа №1", "SHORT_NAME": "СОШ 1"},
          "PARTICIPANT": {"SYS_GUID": " student-guid ", "SURNAME": "Петров", "NAME": "Иван", "SECONDNAME": "Сергеевич",
                          "GRADE": {"NAME": "9А", "SCHOOL": {"NAME": "Школа №1"}}}
        }]
      }
    }"#;

    const PARENT: &str = r#"{
      "success": true,
      "data": {
        "SURNAME": "Петрова", "NAME": "Анна", "SECONDNAME": "Игоревна",
        "SCHOOLS": [{
          "ROLES": ["parents"],
          "SCHOOL": {"NAME": "Школа №1"},
          "PARENT": {"SURNAME": "Петрова", "NAME": "Анна", "SECONDNAME": "Игоревна"},
          "USER_PARTICIPANTS": [
            {"SYS_GUID": "c1", "SURNAME": "Петров", "NAME": "Иван", "GRADE": {"NAME": "9А", "SCHOOL": {"NAME": "Школа №1"}}},
            {"SYS_GUID": "c2", "SURNAME": "Петрова", "NAME": "Мария", "SECONDNAME": "Ивановна", "GRADE": {"NAME": "5Б"}}
          ]
        }]
      }
    }"#;

    fn resp(json: &str) -> LoginResponse {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parses_student() {
        let r = resp(&STUDENT.replace("student-guid", " student-guid "));
        match account_from(&r).unwrap() {
            LoginAccount::Student { guid } => assert_eq!(guid, "student-guid"),
            _ => panic!("ожидался ученик"),
        }
        let s = build_student_session("sid", "key", &r).unwrap();
        assert_eq!(s.user_guid, "student-guid");
        assert_eq!(s.full_name, "Петров Иван Сергеевич");
        assert_eq!(s.school_name, "Школа №1");
        assert_eq!(s.school_class, "9А");
        assert!(!s.is_parent);
    }

    #[test]
    fn parses_parent_and_children() {
        let r = resp(PARENT);
        match account_from(&r).unwrap() {
            LoginAccount::Parent {
                children,
                parent_name,
            } => {
                assert_eq!(parent_name, "Петрова Анна Игоревна");
                assert_eq!(children.len(), 2);
                assert_eq!(children[0].school_class, "9А");
                assert_eq!(children[0].school_name, "Школа №1");
                // У второго ребёнка школы в GRADE нет — берётся школа записи.
                assert_eq!(children[1].school_name, "Школа №1");
                assert_eq!(children[1].full_name, "Петрова Мария Ивановна");
            }
            _ => panic!("ожидался родитель"),
        }
    }

    #[test]
    fn rejects_unsuccessful_and_empty_data() {
        let r: LoginResponse =
            serde_json::from_str(r#"{"success":false,"message":"bad"}"#).unwrap();
        assert_eq!(
            account_from(&r).unwrap_err(),
            LoginError::ServerRejected("bad".into())
        );
        let r: LoginResponse = serde_json::from_str(r#"{"success":true}"#).unwrap();
        assert_eq!(account_from(&r).unwrap_err(), LoginError::BadData);
    }

    #[test]
    fn parses_init_key() {
        let ok = HttpResponse {
            status: 200,
            body: br#"{"key":"abc"}"#.to_vec(),
        };
        assert_eq!(parse_init_http(ok).unwrap(), "abc");
        let empty = HttpResponse {
            status: 200,
            body: br#"{"status":"no","key":""}"#.to_vec(),
        };
        assert!(matches!(
            parse_init_http(empty),
            Err(LoginError::ServerRejected(_))
        ));
        let garbage = HttpResponse {
            status: 200,
            body: b"oops".to_vec(),
        };
        assert!(matches!(
            parse_init_http(garbage),
            Err(LoginError::ServerRejected(_))
        ));
    }
}
