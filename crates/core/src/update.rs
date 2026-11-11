use serde::Deserialize;

use crate::api::ApiRequest;

// Android: релиз с пометкой latest. Она закреплена за Android-релизами
// (iOS публикуется с make_latest: false), поэтому /releases/latest всегда
// указывает на последний android-vX.Y.Z. GitHub сам пропускает черновики и
// пре-релизы.
const ANDROID_LATEST_URL: &str =
    "https://api.github.com/repos/vodolazny/burmalda57/releases/latest";

// iOS: один плавающий релиз ios-latest, запрашивается напрямую по тегу.
// Версия берётся из названия релиза («iOS latest build 1.0.1 (unsigned)»)
// или, если её там нет, из описания («Версия: 1.0.1») — см. ios.yml.
const IOS_LATEST_URL: &str =
    "https://api.github.com/repos/vodolazny/burmalda57/releases/tags/ios-latest";

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}

/// Запрос для платформы (`pkg_ext` — ".apk" или ".ipa").
pub fn request(pkg_ext: &str) -> ApiRequest {
    match pkg_ext {
        ".ipa" => ApiRequest::get(IOS_LATEST_URL, 10),
        _ => ApiRequest::get(ANDROID_LATEST_URL, 10),
    }
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        // Не-semver версию не считаем обновлением: иначе любая отличающаяся
        // (в т.ч. откат релиза) показывала бы ложный баннер.
        _ => false,
    }
}

/// Версия из тега Android-релиза. Понимает и старые теги `vX.Y.Z`,
/// выпущенные до перехода на монорепо.
fn android_version(tag: &str) -> Option<&str> {
    tag.strip_prefix("android-v")
        .or_else(|| tag.strip_prefix('v'))
}

/// Первая semver-версия среди слов текста (знаки препинания по краям слова
/// отбрасываются): «iOS latest build 1.0.1 (unsigned)» → 1.0.1.
fn version_in(text: &str) -> Option<semver::Version> {
    text.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        semver::Version::parse(word).ok()
    })
}

/// Ссылка на пакет из релиза; без подходящего файла — страница релиза.
fn download_url(rel: GhRelease, matches_asset: impl Fn(&str) -> bool) -> String {
    rel.assets
        .into_iter()
        .find(|a| matches_asset(&a.name))
        .map(|a| a.browser_download_url)
        .unwrap_or(rel.html_url)
}

/// Android: ответ `/releases/latest` (один объект).
fn parse_android_latest(raw: &str, current: &str) -> Option<UpdateInfo> {
    let rel: GhRelease = serde_json::from_str(raw).ok()?;
    if rel.draft || rel.prerelease {
        return None;
    }
    // latest, указывающий на не-Android релиз, обновлением не считается.
    let latest = android_version(&rel.tag_name)?.to_string();
    if !is_newer(&latest, current) {
        return None;
    }
    Some(UpdateInfo {
        version: latest,
        url: download_url(rel, |name| {
            name.ends_with(".apk") && !name.starts_with("burmalda57-rustore-")
        }),
    })
}

/// iOS: ответ GitHub по релизу `ios-latest` (один объект).
fn parse_ios_latest(raw: &str, current: &str) -> Option<UpdateInfo> {
    let rel: GhRelease = serde_json::from_str(raw).ok()?;
    if rel.draft || rel.prerelease {
        return None;
    }
    let latest = rel
        .name
        .as_deref()
        .and_then(version_in)
        .or_else(|| rel.body.as_deref().and_then(version_in))?
        .to_string();
    if !is_newer(&latest, current) {
        return None;
    }
    Some(UpdateInfo {
        version: latest,
        url: download_url(rel, |name| name.ends_with(".ipa")),
    })
}

/// Разобрать ответ GitHub на запрос из [`request`]. `pkg_ext` — ".apk"
/// или ".ipa".
pub fn parse_update(raw: &str, current: &str, pkg_ext: &str) -> Option<UpdateInfo> {
    match pkg_ext {
        ".apk" => parse_android_latest(raw, current),
        ".ipa" => parse_ios_latest(raw, current),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANDROID_LATEST: &str = r#"{
        "tag_name": "android-v1.8.0",
        "name": "burmalda57 android-v1.8.0",
        "html_url": "https://x/android-1.8.0",
        "assets": [
            {"name": "burmalda57-rustore-1.8.0.apk", "browser_download_url": "https://x/rustore.apk"},
            {"name": "burmalda57-1.8.0.apk", "browser_download_url": "https://x/app-1.8.0.apk"}
        ]
    }"#;

    const IOS_LATEST: &str = r#"{
        "tag_name": "ios-latest",
        "name": "iOS latest build 1.0.1 (unsigned)",
        "html_url": "https://x/ios-latest",
        "body": "- Версия: 1.0.1",
        "assets": [
            {"name": "burmalda57-ios-latest-unsigned.ipa", "browser_download_url": "https://x/latest.ipa"}
        ]
    }"#;

    #[test]
    fn android_reads_version_from_tag() {
        let u = parse_update(ANDROID_LATEST, "1.7.3", ".apk").unwrap();
        assert_eq!(u.version, "1.8.0");
        assert_eq!(u.url, "https://x/app-1.8.0.apk");
        assert!(parse_update(ANDROID_LATEST, "1.8.0", ".apk").is_none());
        assert!(parse_update(ANDROID_LATEST, "1.9.0", ".apk").is_none());
    }

    #[test]
    fn android_accepts_legacy_tag() {
        let raw = r#"{
            "tag_name": "v2.0.0",
            "html_url": "https://x/android-2.0.0",
            "assets": [
                {"name": "app-2.0.0.apk", "browser_download_url": "https://x/app-2.0.0.apk"}
            ]
        }"#;
        assert_eq!(parse_update(raw, "1.7.3", ".apk").unwrap().version, "2.0.0");
    }

    #[test]
    fn android_falls_back_to_release_page_without_asset() {
        let raw = r#"{"tag_name": "android-v2.0.0", "html_url": "https://x/page", "assets": []}"#;
        assert_eq!(parse_update(raw, "1.7.3", ".apk").unwrap().url, "https://x/page");
    }

    #[test]
    fn android_ignores_drafts_prereleases_and_other_platforms() {
        let draft = ANDROID_LATEST.replace("\"tag_name\"", "\"draft\": true, \"tag_name\"");
        assert!(parse_update(&draft, "1.7.3", ".apk").is_none());
        let pre = ANDROID_LATEST.replace("\"tag_name\"", "\"prerelease\": true, \"tag_name\"");
        assert!(parse_update(&pre, "1.7.3", ".apk").is_none());
        // latest указывает на iOS-релиз, а не на Android
        assert!(parse_update(IOS_LATEST, "1.0.0", ".apk").is_none());
        assert!(parse_update(r#"{"message": "Not Found"}"#, "1.7.3", ".apk").is_none());
    }

    #[test]
    fn ios_reads_version_from_release_name() {
        let u = parse_update(IOS_LATEST, "1.0.0", ".ipa").unwrap();
        assert_eq!(u.version, "1.0.1");
        assert_eq!(u.url, "https://x/latest.ipa");
        assert!(parse_update(IOS_LATEST, "1.0.1", ".ipa").is_none());
        assert!(parse_update(IOS_LATEST, "1.0.2", ".ipa").is_none());
    }

    #[test]
    fn ios_falls_back_to_release_body() {
        let raw = r#"{
            "tag_name": "ios-latest",
            "name": "iOS latest build",
            "html_url": "https://x/ios-latest",
            "body": "- Версия: 1.2.0",
            "assets": []
        }"#;
        let u = parse_update(raw, "1.0.0", ".ipa").unwrap();
        assert_eq!(u.version, "1.2.0");
        assert_eq!(u.url, "https://x/ios-latest");
    }

    #[test]
    fn ios_ignores_unusable_responses() {
        let draft = IOS_LATEST.replace("\"tag_name\"", "\"draft\": true, \"tag_name\"");
        assert!(parse_update(&draft, "1.0.0", ".ipa").is_none());
        let no_version = r#"{"tag_name": "ios-latest", "name": "iOS latest", "html_url": "https://x"}"#;
        assert!(parse_update(no_version, "1.0.0", ".ipa").is_none());
        assert!(parse_update(r#"{"message": "Not Found"}"#, "1.0.0", ".ipa").is_none());
        assert!(parse_update("[]", "1.0.0", ".ipa").is_none());
    }

    #[test]
    fn request_depends_on_platform() {
        assert!(request(".apk").url.ends_with("/releases/latest"));
        assert!(request(".ipa").url.ends_with("/releases/tags/ios-latest"));
    }

    #[test]
    fn version_comparison_is_semver_only() {
        assert!(is_newer("1.7.3", "1.7.2"));
        assert!(!is_newer("1.7.3", "1.7.3"));
        assert!(!is_newer("nightly", "1.7.2"));
        assert!(!is_newer("1.7.2", "nightly"));
    }
}
