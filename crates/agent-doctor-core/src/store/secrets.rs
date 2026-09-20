//! Local-file secrets for Agent Doctor (no OS Keychain / Credential Manager prompts).
//!
//! Default backend writes `{config_dir}/agent-doctor/secrets.json` (mode `0600` on Unix).
//! On Windows the same map is DPAPI-encrypted at `secrets.dpapi` so CredMan never opens.
//!
//! Optional:
//! - `AGENT_DOCTOR_SECRETS_BACKEND=memory` — unit tests / CI
//! - `AGENT_DOCTOR_SECRETS_BACKEND=keyring` — legacy OS credential store (may prompt)
//!
//! Keys previously stored in the OS keyring are **not** auto-pulled (that would reintroduce
//! unlock prompts). Re-enter once in the app, or set `keyring` backend temporarily.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(windows)]
use anyhow::bail;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const KEYRING_SERVICE: &str = "agent-doctor";

pub const SECRET_TEAM_API_KEY: &str = "team.api_key";
pub const SECRET_TEAM_ENGINE_INGEST: &str = "team.engine_ingest_token";
pub const SECRET_TEAMUPS_LICENSE: &str = "teamups.license";
pub const SECRET_CUSTOM_SKILLS_TOKEN: &str = "skills.custom.token";
pub const SECRET_OVERLAY_API_KEY: &str = "overlay.active_api_key";

pub fn personal_api_key_account(provider_id: &str) -> String {
    format!("personal.{provider_id}.api_key")
}

/// Human-readable name of the active secret store (for errors / docs).
pub fn platform_secret_store_name() -> &'static str {
    if std::env::var("AGENT_DOCTOR_SECRETS_BACKEND")
        .ok()
        .is_some_and(|v| v.eq_ignore_ascii_case("keyring"))
    {
        #[cfg(target_os = "macos")]
        {
            return "Keychain";
        }
        #[cfg(target_os = "windows")]
        {
            return "Windows Credential Manager";
        }
        #[cfg(target_os = "linux")]
        {
            return "Secret Service";
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            return "OS credential store";
        }
    }
    #[cfg(windows)]
    {
        "local DPAPI secrets file"
    }
    #[cfg(not(windows))]
    {
        "local secrets file"
    }
}

pub trait SecretBackend: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn delete(&self, account: &str) -> Result<()>;
}

fn secrets_json_path() -> Result<PathBuf> {
    let dir = crate::store::agent_doctor_config_dir()
        .context("could not resolve agent-doctor config dir")?;
    Ok(dir.join("secrets.json"))
}

#[cfg(windows)]
fn secrets_dpapi_path() -> Result<PathBuf> {
    let dir = crate::store::agent_doctor_config_dir()
        .context("could not resolve agent-doctor config dir")?;
    Ok(dir.join("secrets.dpapi"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    secrets: HashMap<String, String>,
}

/// Local JSON vault (`secrets.json`, mode 0600). Default on macOS / Linux.
pub struct LocalFileSecretBackend {
    path: PathBuf,
    lock: Mutex<()>,
}

impl LocalFileSecretBackend {
    pub fn open_default() -> Result<Self> {
        Ok(Self {
            path: secrets_json_path()?,
            lock: Mutex::new(()),
        })
    }

    pub fn open_at(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load_unlocked(&self) -> Result<SecretsFile> {
        if !self.path.exists() {
            return Ok(SecretsFile::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        if raw.trim().is_empty() {
            return Ok(SecretsFile::default());
        }
        serde_json::from_str(&raw)
            .with_context(|| format!("invalid secrets JSON at {}", self.path.display()))
    }

    fn save_unlocked(&self, file: &SecretsFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(file)? + "\n";
        std::fs::write(&self.path, raw)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

impl SecretBackend for LocalFileSecretBackend {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("secrets file mutex");
        let mut file = self.load_unlocked()?;
        file.secrets.insert(account.to_string(), secret.to_string());
        self.save_unlocked(&file)
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().expect("secrets file mutex");
        Ok(self.load_unlocked()?.secrets.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("secrets file mutex");
        let mut file = self.load_unlocked()?;
        file.secrets.remove(account);
        self.save_unlocked(&file)
    }
}

/// Production backend using the platform credential store via `keyring` (opt-in).
pub struct KeyringBackend;

impl SecretBackend for KeyringBackend {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let store = "OS credential store";
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .with_context(|| format!("failed to open {store} entry for `{account}`"))?;
        entry
            .set_password(secret)
            .with_context(|| format!("failed to store secret `{account}` in {store}"))
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let store = "OS credential store";
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .with_context(|| format!("failed to open {store} entry for `{account}`"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => {
                Err(err).with_context(|| format!("failed to read secret `{account}` from {store}"))
            }
        }
    }

    fn delete(&self, account: &str) -> Result<()> {
        let store = "OS credential store";
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .with_context(|| format!("failed to open {store} entry for `{account}`"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err)
                .with_context(|| format!("failed to delete secret `{account}` from {store}")),
        }
    }
}

/// Legacy composite: OS keyring first, Windows DPAPI file fallback.
pub struct CompositeSecretBackend {
    primary: KeyringBackend,
    #[cfg(windows)]
    windows_fallback: WindowsDpapiVault,
}

impl CompositeSecretBackend {
    pub fn new() -> Self {
        Self {
            primary: KeyringBackend,
            #[cfg(windows)]
            windows_fallback: WindowsDpapiVault::default_path(),
        }
    }
}

impl Default for CompositeSecretBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for CompositeSecretBackend {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        match self.primary.set(account, secret) {
            Ok(()) => {
                #[cfg(windows)]
                {
                    let _ = self.windows_fallback.delete(account);
                }
                Ok(())
            }
            Err(primary_err) => {
                #[cfg(windows)]
                {
                    self.windows_fallback
                        .set(account, secret)
                        .with_context(|| {
                            format!(
                                "{primary_err:#}; also failed DPAPI file fallback \
                                 (Windows Credential Manager unavailable)"
                            )
                        })?;
                    return Ok(());
                }
                #[cfg(not(windows))]
                {
                    Err(primary_err)
                }
            }
        }
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        match self.primary.get(account) {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => {
                #[cfg(windows)]
                {
                    return self.windows_fallback.get(account);
                }
                #[cfg(not(windows))]
                {
                    Ok(None)
                }
            }
            Err(primary_err) => {
                #[cfg(windows)]
                {
                    match self.windows_fallback.get(account) {
                        Ok(Some(value)) => Ok(Some(value)),
                        Ok(None) => Err(primary_err),
                        Err(fallback_err) => Err(primary_err).context(fallback_err),
                    }
                }
                #[cfg(not(windows))]
                {
                    Err(primary_err)
                }
            }
        }
    }

    fn delete(&self, account: &str) -> Result<()> {
        let primary = self.primary.delete(account);
        #[cfg(windows)]
        {
            let fallback = self.windows_fallback.delete(account);
            primary.or(fallback)?;
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            primary
        }
    }
}

/// In-memory backend for unit tests (never used for production secrets).
#[derive(Clone, Default)]
pub struct MemorySecretBackend {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl MemorySecretBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretBackend for MemorySecretBackend {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.inner
            .lock()
            .expect("secret mutex")
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self
            .inner
            .lock()
            .expect("secret mutex")
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.inner.lock().expect("secret mutex").remove(account);
        Ok(())
    }
}

pub fn default_secret_backend() -> Arc<dyn SecretBackend> {
    let mode = std::env::var("AGENT_DOCTOR_SECRETS_BACKEND")
        .unwrap_or_default()
        .to_ascii_lowercase();
    match mode.as_str() {
        "memory" => {
            static MEMORY: OnceLock<Arc<MemorySecretBackend>> = OnceLock::new();
            MEMORY
                .get_or_init(|| Arc::new(MemorySecretBackend::new()))
                .clone()
        }
        "keyring" => Arc::new(CompositeSecretBackend::new()),
        _ => {
            // Default: local file — no Keychain / CredMan / Secret Service prompts.
            #[cfg(windows)]
            {
                static WINDOWS: OnceLock<Arc<WindowsDpapiVault>> = OnceLock::new();
                WINDOWS
                    .get_or_init(|| Arc::new(WindowsDpapiVault::default_path()))
                    .clone()
            }
            #[cfg(not(windows))]
            {
                static LOCAL: OnceLock<Arc<LocalFileSecretBackend>> = OnceLock::new();
                LOCAL
                    .get_or_init(|| {
                        Arc::new(LocalFileSecretBackend::open_default().unwrap_or_else(|_| {
                            LocalFileSecretBackend::open_at(PathBuf::from("secrets.json"))
                        }))
                    })
                    .clone()
            }
        }
    }
}

// --- Windows DPAPI local vault (default on Windows; no Credential Manager) ----

#[cfg(windows)]
struct WindowsDpapiVault {
    path: PathBuf,
    lock: Mutex<()>,
}

#[cfg(windows)]
impl WindowsDpapiVault {
    fn default_path() -> Self {
        let path = secrets_dpapi_path().unwrap_or_else(|_| PathBuf::from("secrets.dpapi"));
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load_map(&self) -> Result<HashMap<String, String>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let protected = std::fs::read(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        let plain = dpapi_unprotect(&protected)?;
        let map: HashMap<String, String> =
            serde_json::from_slice(&plain).context("invalid DPAPI secrets vault JSON")?;
        Ok(map)
    }

    fn save_map(&self, map: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let plain = serde_json::to_vec(map)?;
        let protected = dpapi_protect(&plain)?;
        std::fs::write(&self.path, protected)?;
        Ok(())
    }
}

#[cfg(windows)]
impl SecretBackend for WindowsDpapiVault {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("dpapi mutex");
        let mut map = self.load_map()?;
        map.insert(account.to_string(), secret.to_string());
        self.save_map(&map)
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().expect("dpapi mutex");
        Ok(self.load_map()?.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("dpapi mutex");
        let mut map = self.load_map()?;
        map.remove(account);
        if map.is_empty() {
            if self.path.exists() {
                std::fs::remove_file(&self.path)?;
            }
            return Ok(());
        }
        self.save_map(&map)
    }
}

#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // SAFETY: CryptProtectData writes an allocated blob into data_out; we copy then LocalFree.
    let ok = unsafe {
        CryptProtectData(
            &mut data_in,
            PWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
    };
    if ok.is_err() {
        bail!("CryptProtectData failed — cannot encrypt secrets for local vault");
    }
    let slice = unsafe { std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize) };
    let out = slice.to_vec();
    unsafe {
        let _ = LocalFree(HLOCAL(data_out.pbData as _));
    }
    Ok(out)
}

#[cfg(windows)]
fn dpapi_unprotect(protected: &[u8]) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &mut data_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
    };
    if ok.is_err() {
        bail!(
            "CryptUnprotectData failed — local DPAPI secrets vault unreadable for this Windows user"
        );
    }
    let slice = unsafe { std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize) };
    let out = slice.to_vec();
    unsafe {
        let _ = LocalFree(HLOCAL(data_out.pbData as _));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn local_file_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let backend = LocalFileSecretBackend::open_at(path.clone());
        backend.set("team.api_key", "evk_test").unwrap();
        assert_eq!(
            backend.get("team.api_key").unwrap().as_deref(),
            Some("evk_test")
        );
        backend.delete("team.api_key").unwrap();
        assert_eq!(backend.get("team.api_key").unwrap(), None);
        assert!(path.exists());
    }
}
