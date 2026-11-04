// Exercise the platform-independent mobile storage contract without a Slint backend.
use burmalda57_core::SecureStore;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod platform {
    pub trait Platform: Send + Sync {
        fn wrap_key(&self, data: &[u8]) -> Result<Vec<u8>, String>;
        fn unwrap_key(&self, data: &[u8]) -> Result<Vec<u8>, String>;
    }
}
#[path = "../../mobile/src/store.rs"]
mod file_store;

#[derive(Default)]
struct Keys {
    empty_wrap: AtomicBool,
    temporary_error: AtomicBool,
}
impl platform::Platform for Keys {
    fn wrap_key(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        Ok(if self.empty_wrap.load(Ordering::SeqCst) {
            vec![]
        } else {
            data.to_vec()
        })
    }
    fn unwrap_key(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if self.temporary_error.load(Ordering::SeqCst) {
            Err("provider unavailable".into())
        } else {
            Ok(data.to_vec())
        }
    }
}
fn directory() -> String {
    let path = std::env::temp_dir().join(format!("b57-store-{:032x}", rand::random::<u128>()));
    std::fs::create_dir_all(&path).unwrap();
    path.to_string_lossy().into_owned()
}
#[test]
fn rejects_empty_wrapped_key_without_reporting_a_saved_session() {
    let dir = directory();
    let keys = Arc::new(Keys::default());
    keys.empty_wrap.store(true, Ordering::SeqCst);
    let store = file_store::FileStore::new(dir.clone(), keys);
    assert!(store.save("", ".session", b"session", "data").is_err());
    assert!(!std::path::Path::new(&dir).join(".dek").exists());
    assert!(!std::path::Path::new(&dir).join(".session").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn temporary_key_error_preserves_the_key_and_existing_data() {
    let dir = directory();
    let keys = Arc::new(Keys::default());
    let store = file_store::FileStore::new(dir.clone(), keys.clone());
    store.save("", ".notes", b"notes", "keep this").unwrap();
    let original = std::fs::read(format!("{dir}/.dek")).unwrap();
    keys.temporary_error.store(true, Ordering::SeqCst);
    let restarted = file_store::FileStore::new(dir.clone(), keys.clone());
    assert!(restarted.save("", ".session", b"session", "new").is_err());
    assert_eq!(std::fs::read(format!("{dir}/.dek")).unwrap(), original);
    keys.temporary_error.store(false, Ordering::SeqCst);
    assert_eq!(
        restarted.load("", ".notes", b"notes").as_deref(),
        Some("keep this")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn atomic_replacement_survives_a_new_store() {
    let dir = directory();
    let keys = Arc::new(Keys::default());
    let store = file_store::FileStore::new(dir.clone(), keys.clone());
    store.save("", ".notes", b"notes", "first").unwrap();
    store.save("", ".notes", b"notes", "second").unwrap();
    let restarted = file_store::FileStore::new(dir.clone(), keys);
    assert_eq!(
        restarted.load("", ".notes", b"notes").as_deref(),
        Some("second")
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}
