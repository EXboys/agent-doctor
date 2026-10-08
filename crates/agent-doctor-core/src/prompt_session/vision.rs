//! Which Ask turns can carry real pictures, and how a picture is read for sending.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::Serialize;

use super::env::{collect_overlay_env, resolve_claude_overlay, resolve_codex_overlay};
use crate::profile::{PROVIDER_KIND_ENV, PROVIDER_KIND_PERSONAL};
use crate::setup::MODEL_ENV;

/// Formats every vision runtime we send to accepts (DeepSeek, OpenAI, Anthropic).
pub const SENDABLE_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Inline images are base64 in the request body; keep well under provider limits.
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Longest edge after we shrink. Enough for charts and screens; matches common API advice.
const MAX_SEND_EDGE: u32 = 1600;
const JPEG_QUALITY: u8 = 85;
/// Already this small and within the edge: leave the file alone.
const SKIP_SHRINK_BYTES: usize = 400 * 1024;

const VISION_RUNTIMES: &[&str] = &["codex", "claude-code", "deepseek-harness"];

#[derive(Debug, Clone, Serialize)]
pub struct AskImageSupport {
    pub runtime: String,
    pub sees_images: bool,
    pub model: Option<String>,
    pub formats: Vec<String>,
}

/// Whether Ask on `runtime` can hand pictures to the model this turn.
pub fn ask_image_support(runtime: &str) -> AskImageSupport {
    let runtime = crate::evotown::normalize_runtime(runtime);
    let overlay = collect_overlay_env();
    AskImageSupport {
        sees_images: runtime_sees_images(&runtime, &overlay),
        model: overlay_model(&overlay),
        formats: SENDABLE_IMAGE_EXTS.iter().map(|s| s.to_string()).collect(),
        runtime,
    }
}

fn overlay_model(overlay: &HashMap<String, String>) -> Option<String> {
    overlay
        .get(MODEL_ENV)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Model ids known to accept image input. Unknown ids count as text-only so a
/// picture never turns into a failed reply.
pub(crate) fn model_sees_images(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    let id = id.rsplit('/').next().unwrap_or(&id);
    if id.is_empty() {
        return false;
    }
    // DeepSeek: V4.1 Flash is natively multimodal; the old flash names route to it.
    // `deepseek-chat` / `deepseek-reasoner` / `deepseek-v4-pro` stay text-only.
    if id.starts_with("deepseek") {
        return id.starts_with("deepseek-flash")
            || id.starts_with("deepseek-v4-flash")
            || id.contains("vision")
            || id.contains("-vl");
    }
    const PREFIXES: &[&str] = &[
        "gpt-4o",
        "gpt-4.1",
        "gpt-5",
        "o3",
        "o4",
        "chatgpt-4o",
        "claude",
        "gemini",
        "grok-4",
        "qwen-vl",
        "qwen2.5-vl",
        "qwen3-vl",
        "glm-4v",
        "glm-4.5v",
        "kimi-vl",
        "pixtral",
    ];
    if PREFIXES.iter().any(|p| id.starts_with(p)) {
        return true;
    }
    id.contains("vision") || id.contains("-vl") || id.contains("omni")
}

/// Runtime + active provider decide. Native Codex/Claude logins see images; a
/// personal or company gateway sees images only when its model does.
pub(crate) fn runtime_sees_images(runtime: &str, overlay: &HashMap<String, String>) -> bool {
    if !VISION_RUNTIMES.contains(&runtime) {
        return false;
    }
    let model = overlay_model(overlay);
    let personal = overlay
        .get(PROVIDER_KIND_ENV)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(PROVIDER_KIND_PERSONAL));
    let gateway = match runtime {
        "codex" => resolve_codex_overlay(overlay).is_some(),
        "claude-code" => resolve_claude_overlay(overlay).is_some(),
        _ => personal || model.is_some(),
    };
    if !gateway {
        // Own login: Codex / Claude models see images. DeepSeek Harness still
        // checks the ACP `promptCapabilities.image` flag before sending.
        return true;
    }
    model.as_deref().is_some_and(model_sees_images)
}

pub(crate) fn is_sendable_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    if !SENDABLE_IMAGE_EXTS.contains(&ext.as_str()) {
        return false;
    }
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_IMAGE_BYTES)
}

/// Pictures this turn will carry for `runtime`. Empty when the model cannot see them.
pub(crate) fn turn_images(
    runtime: &str,
    paths: &[PathBuf],
    overlay: &HashMap<String, String>,
) -> Vec<PathBuf> {
    if paths.is_empty() || !runtime_sees_images(runtime, overlay) {
        return Vec::new();
    }
    paths
        .iter()
        .filter(|p| is_sendable_image(p))
        .cloned()
        .collect()
}

/// Codex reads a file path. Shrink first, keep the temp file until the turn ends.
pub(crate) struct PreparedTurnImages {
    pub paths: Vec<PathBuf>,
    pub keep: Vec<tempfile::NamedTempFile>,
}

pub(crate) fn prepare_turn_images(
    runtime: &str,
    paths: &[PathBuf],
    overlay: &HashMap<String, String>,
) -> PreparedTurnImages {
    let mut out = PreparedTurnImages {
        paths: Vec::new(),
        keep: Vec::new(),
    };
    for path in turn_images(runtime, paths, overlay) {
        match write_shrunk(&path) {
            Some((ready, Some(tmp))) => {
                out.paths.push(ready);
                out.keep.push(tmp);
            }
            Some((ready, None)) => out.paths.push(ready),
            None => {}
        }
    }
    out
}

/// Media type from the file's first bytes (providers check content, not the name).
pub(crate) fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub(crate) struct EncodedImage {
    pub media_type: &'static str,
    pub base64: String,
}

pub(crate) fn encode_image(path: &Path) -> Option<EncodedImage> {
    let (bytes, media_type) = load_for_send(path)?;
    Some(EncodedImage {
        media_type,
        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

fn load_for_send(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    if !is_sendable_image(path) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let media = sniff_media_type(&bytes)?;
    Some(shrink_bytes(&bytes, media))
}

fn write_shrunk(path: &Path) -> Option<(PathBuf, Option<tempfile::NamedTempFile>)> {
    let (bytes, media) = load_for_send(path)?;
    let original = std::fs::read(path).ok()?;
    if bytes == original {
        return Some((path.to_path_buf(), None));
    }
    let ext = match media {
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        _ => ".jpg",
    };
    let mut tmp = tempfile::Builder::new()
        .prefix("ad-ask-")
        .suffix(ext)
        .tempfile()
        .ok()?;
    use std::io::Write;
    tmp.write_all(&bytes).ok()?;
    Some((tmp.path().to_path_buf(), Some(tmp)))
}

fn shrink_bytes(bytes: &[u8], media: &'static str) -> (Vec<u8>, &'static str) {
    try_shrink(bytes, media).unwrap_or_else(|| (bytes.to_vec(), media))
}

fn try_shrink(bytes: &[u8], media: &'static str) -> Option<(Vec<u8>, &'static str)> {
    // Animated GIF: leave it. Re-encoding drops frames and often grows the file.
    if media == "image/gif" {
        return None;
    }
    if bytes.len() <= SKIP_SHRINK_BYTES {
        if let Ok(img) = image::load_from_memory(bytes) {
            if img.width() <= MAX_SEND_EDGE && img.height() <= MAX_SEND_EDGE {
                return None;
            }
        } else {
            return None;
        }
    }
    let img = image::load_from_memory(bytes).ok()?;
    let (orig_w, orig_h) = (img.width(), img.height());
    let resized = if orig_w > MAX_SEND_EDGE || orig_h > MAX_SEND_EDGE {
        img.resize(
            MAX_SEND_EDGE,
            MAX_SEND_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let keep_png = media == "image/png" && resized.color().has_alpha();
    let out = if keep_png {
        encode_png(&resized)?
    } else {
        encode_jpeg(&resized.to_rgb8())?
    };
    let out_media = if keep_png { "image/png" } else { "image/jpeg" };
    if out.len() >= bytes.len() && resized.width() == orig_w && resized.height() == orig_h {
        return None;
    }
    Some((out, out_media))
}

fn encode_png(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

fn encode_jpeg(rgb: &image::RgbImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::PROVIDER_PROTOCOL_ENV;

    #[test]
    fn deepseek_flash_sees_images_but_pro_and_chat_do_not() {
        assert!(model_sees_images("deepseek-flash"));
        assert!(model_sees_images("deepseek-v4-flash"));
        assert!(model_sees_images("deepseek-v4-flash-vision-exp"));
        assert!(!model_sees_images("deepseek-v4-pro"));
        assert!(!model_sees_images("deepseek-chat"));
        assert!(!model_sees_images("deepseek-reasoner"));
        assert!(model_sees_images("openai/gpt-5.6-terra"));
        assert!(model_sees_images("gemini-3.8-flash"));
        assert!(!model_sees_images("some-unknown-model"));
        assert!(!model_sees_images(""));
    }

    #[test]
    fn gateway_follows_model_and_native_login_sees_images() {
        let empty = HashMap::new();
        assert!(runtime_sees_images("codex", &empty));
        assert!(runtime_sees_images("claude-code", &empty));
        assert!(!runtime_sees_images("hermes", &empty));
        assert!(!runtime_sees_images("openclaw", &empty));

        let mut env = HashMap::new();
        env.insert(PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into());
        env.insert(PROVIDER_PROTOCOL_ENV.into(), "openai".into());
        env.insert("OPENAI_BASE_URL".into(), "https://api.deepseek.com".into());
        env.insert("OPENAI_API_KEY".into(), "sk-test".into());
        env.insert(MODEL_ENV.into(), "deepseek-v4-pro".into());
        assert!(!runtime_sees_images("codex", &env));
        assert!(!runtime_sees_images("deepseek-harness", &env));
        env.insert(MODEL_ENV.into(), "deepseek-flash".into());
        assert!(runtime_sees_images("codex", &env));
        assert!(runtime_sees_images("deepseek-harness", &env));
        env.remove(MODEL_ENV);
        assert!(!runtime_sees_images("codex", &env));
    }

    #[test]
    fn encodes_png_by_content_and_skips_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]).unwrap();
        let encoded = encode_image(&png).expect("png");
        assert_eq!(encoded.media_type, "image/png");
        assert!(!encoded.base64.is_empty());

        let fake = dir.path().join("fake.jpg");
        std::fs::write(&fake, b"not an image").unwrap();
        assert!(encode_image(&fake).is_none());

        let heic = dir.path().join("photo.heic");
        std::fs::write(&heic, b"x").unwrap();
        assert!(!is_sendable_image(&heic));
        assert!(turn_images("hermes", std::slice::from_ref(&png), &HashMap::new()).is_empty());
        assert_eq!(
            turn_images("codex", &[png.clone(), heic], &HashMap::new()),
            vec![png]
        );
    }

    #[test]
    fn shrinks_oversized_rgb_to_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.png");
        let mut img = image::RgbImage::new(1800, 1200);
        for (i, pixel) in img.pixels_mut().enumerate() {
            let n = (i % 251) as u8;
            *pixel = image::Rgb([n, n.wrapping_mul(3), 180]);
        }
        img.save(&path).unwrap();
        let orig = std::fs::metadata(&path).unwrap().len();
        let encoded = encode_image(&path).expect("enc");
        assert_eq!(encoded.media_type, "image/jpeg");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&encoded.base64)
            .unwrap();
        assert!(
            raw.len() < orig as usize,
            "shrunk {} vs orig {orig}",
            raw.len()
        );
        let decoded = image::load_from_memory(&raw).unwrap();
        assert!(decoded.width() <= MAX_SEND_EDGE);
        assert!(decoded.height() <= MAX_SEND_EDGE);

        let prepared = prepare_turn_images("codex", std::slice::from_ref(&path), &HashMap::new());
        assert_eq!(prepared.paths.len(), 1);
        assert_eq!(prepared.keep.len(), 1);
        assert_ne!(prepared.paths[0], path);
    }
}
