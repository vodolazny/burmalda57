// Файловое хранилище с envelope-шифрованием.
//
// DEK — случайные 32 байта, которыми AES-256-GCM шифруются файлы (.session,
// кеши, события...). На диске лежит только обёртка DEK (.dek), ключ-обёртка
// (KEK) не покидает Keystore/Keychain платформы.
//
// Семантика ошибок: временный сбой платформенного KEK — Err, и .dek НЕ
// перезаписывается. Пустой ответ unwrap / битый .dek — Broken, тогда создаётся
// новый DEK (старые файлы уже не прочитать).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use burmalda57_core::{decrypt_blob, encrypt_blob, random_bytes, SecureStore};

use crate::platform::Platform;

const DEK_FILE: &str = ".dek";
static KEY_FILE_LOCK: Mutex<()> = Mutex::new(());

enum DekState {
    Ready([u8; 32]),
    Absent,
    Broken,
}

pub(crate) struct FileStore {
    dir: String,
    platform: Arc<dyn Platform>,
    dek: Mutex<Option<[u8; 32]>>,
}

impl FileStore {
    pub(crate) fn new(dir: String, platform: Arc<dyn Platform>) -> Self {
        Self {
            dir,
            platform,
            dek: Mutex::new(None),
        }
    }

    fn dek_path(&self) -> String {
        format!("{}/{}", self.dir, DEK_FILE)
    }

    fn read_dek(&self) -> Result<DekState, String> {
        if let Some(dek) = *self.dek.lock().unwrap() {
            return Ok(DekState::Ready(dek));
        }
        let path = self.dek_path();
        if !Path::new(&path).exists() {
            return Ok(DekState::Absent);
        }
        let mut blob = Vec::new();
        File::open(&path)
            .and_then(|mut f| f.read_to_end(&mut blob))
            .map_err(|e| format!("Не удалось прочитать .dek: {e}"))?;
        if blob.is_empty() {
            return Ok(DekState::Broken);
        }
        let dek = self.platform.unwrap_key(&blob)?;
        if dek.len() != 32 {
            return Ok(DekState::Broken);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&dek);
        *self.dek.lock().unwrap() = Some(out);
        Ok(DekState::Ready(out))
    }

    fn get_or_create_dek(&self) -> Result<[u8; 32], String> {
        // UI and background polling use separate FileStore instances in one process.
        let _key_lock = KEY_FILE_LOCK.lock().unwrap();
        match self.read_dek()? {
            DekState::Ready(dek) => return Ok(dek),
            DekState::Absent => {}
            DekState::Broken => {
                log::warn!("DEK нечитаем (ключ платформы потерян?) — создаём новый");
            }
        }

        let dek = random_bytes::<32>();
        let blob = self.platform.wrap_key(&dek)?;
        if blob.is_empty() || self.platform.unwrap_key(&blob)?.as_slice() != dek.as_slice() {
            return Err("Не удалось проверить сохранение ключа шифрования".into());
        }
        fs::create_dir_all(&self.dir).map_err(|e| format!("Не удалось создать каталог: {e}"))?;
        let path = self.dek_path();
        atomic_write(&path, &blob)?;
        *self.dek.lock().unwrap() = Some(dek);
        Ok(dek)
    }

    fn existing_dek(&self) -> Option<[u8; 32]> {
        match self.read_dek() {
            Ok(DekState::Ready(dek)) => Some(dek),
            Ok(DekState::Absent) | Ok(DekState::Broken) => None,
            Err(e) => {
                log::warn!("DEK временно недоступен: {e}");
                None
            }
        }
    }
}

impl SecureStore for FileStore {
    fn load(&self, _storage_path: &str, filename: &str, context: &[u8]) -> Option<String> {
        let dek = self.existing_dek()?;
        let path = format!("{}/{}", self.dir, filename);
        let mut blob = Vec::new();
        File::open(path).ok()?.read_to_end(&mut blob).ok()?;
        decrypt_blob(&dek, context, &blob)
    }

    fn save(
        &self,
        _storage_path: &str,
        filename: &str,
        context: &[u8],
        data: &str,
    ) -> Result<(), String> {
        let dek = self.get_or_create_dek()?;
        let blob = encrypt_blob(&dek, context, data)?;
        fs::create_dir_all(&self.dir).map_err(|e| format!("Не удалось создать каталог: {e}"))?;
        let path = format!("{}/{}", self.dir, filename);
        atomic_write(&path, &blob)
    }

    fn delete(&self, _storage_path: &str, filename: &str) {
        let _ = fs::remove_file(format!("{}/{}", self.dir, filename));
    }
}

fn atomic_write(path: &str, bytes: &[u8]) -> Result<(), String> {
    let temporary = format!(
        "{path}.tmp-{:032x}",
        u128::from_le_bytes(random_bytes::<16>())
    );
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        restrict_permissions(path);
        if let Some(parent) = Path::new(path).parent() {
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e: String| format!("Не удалось сохранить файл: {e}"))
}

#[cfg(unix)]
fn restrict_permissions(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &str) {}
