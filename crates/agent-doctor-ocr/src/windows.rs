//! Read printed words with Windows.Media.Ocr (no extra model or key).

use std::path::Path;
use windows::core::{Result, HSTRING};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::StorageFile;

pub fn recognize_text(path: &Path) -> (bool, String) {
    match recognize_inner(path) {
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

fn recognize_inner(path: &Path) -> Result<String> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(storage_path(path)))?.get()?;
    let stream = file.OpenReadAsync()?.get()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let bitmap = decoder
        .GetSoftwareBitmapConvertedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)?
        .get()?;
    let result = ocr_engine()?.RecognizeAsync(&bitmap)?.get()?;
    Ok(result.Text()?.to_string())
}
