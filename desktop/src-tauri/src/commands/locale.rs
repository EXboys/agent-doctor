use tauri::{AppHandle, Emitter, Manager};

/// Interface language shared with the desktop windows. The tray menu reads this
/// too, so the menu bar follows Settings → Language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiLocale {
    En,
    Zh,
}

impl UiLocale {
    pub(crate) fn is_zh(self) -> bool {
        matches!(self, Self::Zh)
    }

    fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }
}

fn locale_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("locale"))
}

/// Saved language, or the system language when the file is not there yet.
pub(crate) fn app_locale(app: &AppHandle) -> UiLocale {
    locale_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| locale_from_saved(&raw))
        .unwrap_or_else(system_locale)
}

pub(crate) fn locale_from_saved(raw: &str) -> Option<UiLocale> {
    match raw.trim() {
        "en" => Some(UiLocale::En),
        "zh" => Some(UiLocale::Zh),
        _ => None,
    }
}

pub(crate) fn locale_from_lang_tag(tag: &str) -> UiLocale {
    let tag = tag.trim().to_ascii_lowercase();
    let primary = tag.split(['.', '@']).next().unwrap_or("");
    if primary.starts_with("zh") {
        UiLocale::Zh
    } else {
        UiLocale::En
    }
}

fn system_locale() -> UiLocale {
    first_system_lang_tag()
        .map(|tag| locale_from_lang_tag(&tag))
        .unwrap_or(UiLocale::En)
}

fn env_lang_tag() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        if value.is_empty() || lower == "c" || lower == "posix" || lower.starts_with("c.") {
            continue;
        }
        return Some(value.to_string());
    }
    None
}

#[cfg(target_os = "macos")]
fn first_system_lang_tag() -> Option<String> {
    macos_preferred_language().or_else(env_lang_tag)
}

#[cfg(target_os = "windows")]
fn first_system_lang_tag() -> Option<String> {
    windows_preferred_language().or_else(env_lang_tag)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn first_system_lang_tag() -> Option<String> {
    env_lang_tag()
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFLocaleCopyPreferredLanguages() -> *const std::ffi::c_void;
    fn CFArrayGetCount(array: *const std::ffi::c_void) -> isize;
    fn CFArrayGetValueAtIndex(
        array: *const std::ffi::c_void,
        index: isize,
    ) -> *const std::ffi::c_void;
    fn CFStringGetCString(
        value: *const std::ffi::c_void,
        buffer: *mut i8,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
fn macos_preferred_language() -> Option<String> {
    // kCFStringEncodingUTF8
    const UTF8: u32 = 0x0800_0100;
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() {
            return None;
        }
        let tag = if CFArrayGetCount(languages) > 0 {
            cfstring_to_string(CFArrayGetValueAtIndex(languages, 0), UTF8)
        } else {
            None
        };
        CFRelease(languages);
        tag
    }
}

#[cfg(target_os = "macos")]
fn cfstring_to_string(value: *const std::ffi::c_void, encoding: u32) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut buf = [0u8; 128];
    let ok =
        unsafe { CFStringGetCString(value, buf.as_mut_ptr().cast(), buf.len() as isize, encoding) };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

#[cfg(target_os = "windows")]
fn windows_preferred_language() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, size: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    let written = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if written <= 1 {
        return None;
    }
    let end = (written as usize - 1).min(buf.len());
    let tag = String::from_utf16_lossy(&buf[..end]);
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

#[tauri::command]
pub fn get_app_locale_command(app: AppHandle) -> Option<String> {
    let raw = std::fs::read_to_string(locale_path(&app).ok()?).ok()?;
    locale_from_saved(&raw).map(|locale| locale.code().to_string())
}

#[tauri::command]
pub fn set_app_locale_command(app: AppHandle, locale: String) -> Result<(), String> {
    if locale != "en" && locale != "zh" {
        return Err("unsupported locale".into());
    }
    let path = locale_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, &locale).map_err(|error| error.to_string())?;
    crate::rebuild_tray_menu(&app);
    crate::update_tray_tooltip(&app);
    let _ = app.emit("locale-changed", locale);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{locale_from_lang_tag, locale_from_saved, UiLocale};

    #[test]
    fn locale_from_saved_accepts_only_en_and_zh() {
        assert_eq!(locale_from_saved("zh"), Some(UiLocale::Zh));
        assert_eq!(locale_from_saved("  en\n"), Some(UiLocale::En));
        assert_eq!(locale_from_saved(""), None);
        assert_eq!(locale_from_saved("zh-CN"), None);
    }

    #[test]
    fn locale_from_lang_tag_treats_chinese_tags_as_zh() {
        assert_eq!(locale_from_lang_tag("zh-Hans-CN"), UiLocale::Zh);
        assert_eq!(locale_from_lang_tag("zh_CN.UTF-8"), UiLocale::Zh);
        assert_eq!(locale_from_lang_tag("en-US"), UiLocale::En);
        assert_eq!(locale_from_lang_tag("C"), UiLocale::En);
    }
}
