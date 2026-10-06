//! Read printed words with Windows.Media.Ocr (no extra model or key).
//!
//! WinRT delivers the completion of these calls back to the calling apartment.
//! `IAsyncOperation::get` then waits on that same thread, so calling it on the
//! UI thread (an STA that is no longer pumping messages) never returns.
//! This module always runs the recognizer on an MTA thread and gives up after
//! a budget so a stuck read cannot hold the send open.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{Error, Result, HRESULT, HSTRING};
use windows::Foundation::{AsyncStatus, IAsyncOperation};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::StorageFile;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

const OCR_BUDGET: Duration = Duration::from_secs(20);

pub fn recognize_text(path: &Path) -> (bool, String) {
    if thread_is_mta() {
        return finish(recognize_inner(path, Instant::now() + OCR_BUDGET));
    }
    let path = path.to_path_buf();
    thread::Builder::new()
        .name("agent-doctor-ocr".into())
        .spawn(move || recognize_off_sta(path))
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or((false, String::new()))
}

fn recognize_off_sta(path: PathBuf) -> (bool, String) {
    if !thread_is_mta() {
        return (false, String::new());
    }
    finish(recognize_inner(&path, Instant::now() + OCR_BUDGET))
}

fn finish(result: Result<String>) -> (bool, String) {
    match result {
        Ok(text) => {
            let text = text.trim().to_string();
            if text.is_empty() {
                (false, String::new())
            } else {
                (true, text)
            }
        }
        Err(_) => (false, String::new()),
    }
}

/// `true` when this thread is (or was just made) an MTA.
/// A pool thread stays MTA for its lifetime; an STA returns false and must hop.
fn thread_is_mta() -> bool {
    thread_local! {
        static MODE: Cell<u8> = const { Cell::new(0) };
    }
    MODE.with(|mode| match mode.get() {
        1 => true,
        2 => false,
        _ => {
            let ok = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };
            mode.set(if ok { 1 } else { 2 });
            ok
        }
    })
}

fn storage_path(path: &Path) -> String {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    // StorageFile rejects `\\?\` extended paths from canonicalize().
    joined.to_string_lossy().replace('/', "\\")
}

fn ocr_engine() -> Result<OcrEngine> {
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(engine);
    }
    let langs = OcrEngine::AvailableRecognizerLanguages()?;
    let size = langs.Size()?;
    for index in 0..size {
        let lang = langs.GetAt(index)?;
        let tag = lang.LanguageTag()?.to_string();
        if tag.starts_with("zh") {
            return OcrEngine::TryCreateFromLanguage(&lang);
        }
    }
    if size > 0 {
        let lang = langs.GetAt(0)?;
        return OcrEngine::TryCreateFromLanguage(&lang);
    }
    for tag in ["zh-Hans", "zh-Hant", "en-US"] {
        let lang = Language::CreateLanguage(&HSTRING::from(tag))?;
        if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                return Ok(engine);
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages()
}

fn wait_async<T: windows::core::RuntimeType + 'static>(
    op: IAsyncOperation<T>,
    deadline: Instant,
) -> Result<T> {
    loop {
        match op.Status()? {
            AsyncStatus::Completed => return op.GetResults(),
            AsyncStatus::Error => {
                let code = op.ErrorCode().unwrap_or(HRESULT(0x8000_4005u32 as i32));
                return Err(Error::from(code));
            }
            AsyncStatus::Canceled => {
                return Err(Error::from(HRESULT(0x8007_04C7u32 as i32)));
            }
            _ => {
                if Instant::now() >= deadline {
                    let _ = op.Cancel();
                    return Err(Error::from(HRESULT(0x8007_05B4u32 as i32)));
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn recognize_inner(path: &Path, deadline: Instant) -> Result<String> {
    let file = wait_async(
        StorageFile::GetFileFromPathAsync(&HSTRING::from(storage_path(path)))?,
        deadline,
    )?;
    let stream = wait_async(file.OpenReadAsync()?, deadline)?;
    let decoder = wait_async(BitmapDecoder::CreateAsync(&stream)?, deadline)?;
    let bitmap = wait_async(
        decoder.GetSoftwareBitmapConvertedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
        )?,
        deadline,
    )?;
    let result = wait_async(ocr_engine()?.RecognizeAsync(&bitmap)?, deadline)?;
    Ok(result.Text()?.to_string())
}
