//! OS credential-store secrets for Agent Doctor.
//!
//! Backends by platform:
//! - macOS: Keychain (`keyring` apple-native)
//! - Windows: Credential Manager (`keyring` windows-native), with DPAPI-encrypted
//!   file fallback when CredMan is unavailable (locked-down / headless sessions)
//! - Linux: Secret Service / keyring (`keyring` linux-native)
//!
//! Failures never silently fall back to plaintext `.env` files.

use std::collections::HashMap;
#[cfg(windows)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use anyhow::bail;
use anyhow::{Context, Result};

pub const KEYRING_SERVICE: &str = "agent-doctor";

pub const SECRET_TEAM_API_KEY: &str = "team.api_key";
pub const SECRET_TEAM_ENGINE_INGEST: &str = "team.engine_ingest_token";
pub const SECRET_TEAMUPS_LICENSE: &str = "teamups.license";
pub const SECRET_CUSTOM_SKILLS_TOKEN: &str = "skills.custom.token";
pub const SECRET_OVERLAY_API_KEY: &str = "overlay.active_api_key";

pub fn personal_api_key_account(provider_id: &str) -> String {
    format!("personal.{provider_id}.api_key")
}

/// Human-readable name of the OS secret store (for errors / docs).
pub fn platform_secret_store_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Keychain"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows Credential Manager"
    }
    #[cfg(target_os = "linux")]
    {
        "Secret Service"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "OS credential store"
    }
}

pub trait SecretBackend: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn delete(&self, account: &str) -> Result<()>;
}

/// Production backend using the platform credential store via `keyring`.
pub struct KeyringBackend;

impl SecretBackend for KeyringBackend {
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let store = platform_secret_store_name();
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .with_context(|| format!("failed to open {store} entry for `{account}`"))?;
        entry
            .set_password(secret)
            .with_context(|| format!("failed to store secret `{account}` in {store}"))
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let store = platform_secret_store_name();
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
        let store = platform_secret_store_name();
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

/// Try OS credential store first; on Windows, fall back to DPAPI-encrypted local vault.
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
                // Best-effort: drop any stale Windows fallback copy after CredMan succeeds.
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
    Arc::new(CompositeSecretBackend::new())
}

// --- Windows DPAPI encrypted vault (Credential Manager fallback) ---------------

#[cfg(windows)]
struct WindowsDpapiVault {
    path: PathBuf,
}

#[cfg(windows)]
impl WindowsDpapiVault {
    fn default_path() -> Self {
        let path = dirs::config_dir()
            .map(|base| base.join("agent-doctor").join("secrets.dpapi"))
            .unwrap_or_else(|| PathBuf::from("secrets.dpapi"));
        Self { path }
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

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let mut map = self.load_map()?;
        map.insert(account.to_string(), secret.to_string());
        self.save_map(&map)
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.load_map()?.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<()> {
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
        bail!("CryptProtectData failed — cannot encrypt secrets for DPAPI fallback");
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
        bail!("CryptUnprotectData failed — DPAPI secrets vault unreadable on this Windows user");
    }
    let slice = unsafe { std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize) };
    let out = slice.to_vec();
    unsafe {
        let _ = LocalFree(HLOCAL(data_out.pbData as _));
    }
    Ok(out)
}
