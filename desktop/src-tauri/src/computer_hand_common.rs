//! The protocol both computer hands speak.
//!
//! The desktop app listens for the computer tool: one JSON request line in,
//! one JSON reply line out. Tool names, argument checks and reply shapes live
//! here once so macOS and Windows cannot drift; each platform only implements
//! [`Hand`] with its own way of clicking, typing and reading the screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::computer_geom::picture_changed;

/// Run the hand on its own thread; a stopped hand must not take the app down.
pub(crate) fn spawn(serve: fn() -> Result<()>) {
    thread::Builder::new()
        .name("computer-hand".into())
        .spawn(move || {
            if let Err(err) = serve() {
                eprintln!("computer hand stopped: {err:#}");
            }
        })
        .ok();
}

/// One request line → one `{ok, result}` or `{ok: false, error}` reply line.
pub(crate) fn envelope(result: Result<Value>) -> Value {
    match result {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
    }
}

/// What a platform can actually do. Everything around these calls — tool
/// names, argument checks, reply shapes — is shared.
pub(crate) trait Hand {
    /// `screen_open` through the CUA gateway, before the accessibility path.
    /// `None` falls through to [`Hand::open_named`].
    fn cua_open(&self, args: &Value) -> Option<Value> {
        let _ = args;
        None
    }

    /// The permission check that must pass before any click. macOS prompts
    /// for accessibility; platforms without one are always trusted.
    fn ensure_trusted(&self) -> Result<()> {
        Ok(())
    }

    fn click_at(&self, x: f64, y: f64) -> Result<()>;
    fn press_key(&self, key: &str) -> Result<()>;
    /// The point guard, the wheel event, and the reply.
    fn scroll_view(&self, args: &Value) -> Result<Value>;
    fn type_text(&self, text: &str) -> Result<()>;
    fn menu_text(&self) -> Result<String>;
    fn app_names(&self) -> Result<Vec<String>>;
    fn open_named(&self, app: &str) -> Result<Value>;
    fn list_windows(&self, app: &str) -> Result<Value>;
    fn window_state(&self, app: &str) -> Result<Value>;
    fn click_label(&self, app: &str, label: &str) -> Result<Value>;
    fn set_label(&self, app: &str, label: &str, text: &str) -> Result<Value>;
    fn click_menu(&self, app: &str, names: Vec<String>) -> Result<Value>;
    /// The screen picture and its clickable rows.
    fn look(&self, app: &str) -> Result<Value>;
}

/// One request line, dispatched. This is the whole tool contract.
pub(crate) fn handle_line(hand: &dyn Hand, line: &str) -> Result<Value> {
    let req: Value = serde_json::from_str(line).context("bad request")?;
    let tool = req.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    if let Some(done) = try_cua(hand, tool, &args) {
        return Ok(done);
    }
    hand.ensure_trusted()?;
    match tool {
        "screen_click" => {
            let x = args.get("x").and_then(Value::as_f64).context("missing x")?;
            let y = args.get("y").and_then(Value::as_f64).context("missing y")?;
            hand.click_at(x, y)?;
            Ok(json!({ "clicked": "point", "x": x, "y": y }))
        }
        "screen_press" => {
            let key = args.get("key").and_then(Value::as_str).unwrap_or("");
            hand.press_key(key)?;
            Ok(json!({ "pressed": key }))
        }
        "screen_scroll" => hand.scroll_view(&args),
        "screen_type" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                bail!("text is empty");
            }
            hand.type_text(text)?;
            Ok(json!({ "typed": text.chars().count() }))
        }
        "screen_menus" => Ok(json!({ "text": hand.menu_text()? })),
        "screen_apps" => Ok(json!({ "apps": hand.app_names()? })),
        "screen_open" => hand.open_named(args.get("app").and_then(Value::as_str).unwrap_or("")),
        "screen_windows" => {
            hand.list_windows(args.get("app").and_then(Value::as_str).unwrap_or(""))
        }
        "screen_state" => hand.window_state(args.get("app").and_then(Value::as_str).unwrap_or("")),
        "screen_click_label" => hand.click_label(
            args.get("app").and_then(Value::as_str).unwrap_or(""),
            args.get("label").and_then(Value::as_str).unwrap_or(""),
        ),
        "screen_set" => {
            if !last_has_field() {
                bail!("这个窗口没有输入框。先点一下要填的地方，再用打字。");
            }
            hand.set_label(
                args.get("app").and_then(Value::as_str).unwrap_or(""),
                args.get("label").and_then(Value::as_str).unwrap_or(""),
                args.get("text").and_then(Value::as_str).unwrap_or(""),
            )
        }
        "screen_menu" => hand.click_menu(
            args.get("app").and_then(Value::as_str).unwrap_or(""),
            string_list(args.get("path")),
        ),
        "screen_look" => hand.look(args.get("app").and_then(Value::as_str).unwrap_or("")),
        other => bail!("unknown hand tool: {other}"),
    }
}

/// Tools the CUA gateway answers without touching accessibility. `None`
/// means the normal tool handles it.
fn try_cua(hand: &dyn Hand, tool: &str, args: &Value) -> Option<Value> {
    match tool {
        "screen_apps" => {
            let apps = crate::computer_contrl::cua::running_apps()?;
            Some(json!({ "apps": apps }))
        }
        "screen_open" => hand.cua_open(args),
        "screen_menu" => {
            let names = string_list(args.get("path"));
            let app = args.get("app").and_then(Value::as_str).unwrap_or("");
            if names.is_empty() || !crate::computer_contrl::cua::menu(app, &names) {
                return None;
            }
            Some(json!({ "menu": names }))
        }
        "screen_windows" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or("");
            let windows = crate::computer_contrl::cua::windows(app)?;
            Some(json!({ "windows": windows }))
        }
        _ => None,
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The screenshot behind the last `screen_look`.
pub(crate) struct Shot {
    pub(crate) path: String,
    pub(crate) pixels: (i64, i64),
    pub(crate) points: (i64, i64),
    pub(crate) scale: f64,
}

/// Fingerprint of the newest screenshot, so a click can prove the screen moved.
static LAST_PRINT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
/// Whether the last read saw a text field, which `screen_set` needs.
static LAST_FIELD: AtomicBool = AtomicBool::new(false);

pub(crate) fn remember_print(print: Vec<u8>) {
    *LAST_PRINT.lock().unwrap_or_else(|err| err.into_inner()) = print;
}

pub(crate) fn last_print() -> Vec<u8> {
    LAST_PRINT
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub(crate) fn remember_field(yes: bool) {
    LAST_FIELD.store(yes, Ordering::Relaxed);
}

pub(crate) fn last_has_field() -> bool {
    LAST_FIELD.load(Ordering::Relaxed)
}

/// 48×48 grayscale — enough to tell that the picture changed.
pub(crate) fn shot_fingerprint(path: &str) -> Option<Vec<u8>> {
    let image = image::open(path).ok()?;
    let small = image.resize_exact(48, 48, image::imageops::FilterType::Nearest);
    Some(small.to_luma8().into_raw())
}

/// A click only counts when the screen actually changed. `capture` takes the
/// verification shot; failing to capture keeps the click.
pub(crate) fn confirm_picture(capture: impl FnOnce() -> Result<Shot>) -> Result<()> {
    let before = last_print();
    if before.is_empty() {
        return Ok(());
    }
    thread::sleep(Duration::from_millis(280));
    let shot = match capture() {
        Ok(shot) => shot,
        Err(_) => return Ok(()),
    };
    let after = shot_fingerprint(&shot.path).unwrap_or_default();
    let _ = std::fs::remove_file(&shot.path);
    if picture_changed(&before, &after) {
        remember_print(after);
        Ok(())
    } else {
        bail!("这一下没有点到。画面没有变化。先看一眼屏幕，再点一次。");
    }
}

/// The `screen_look` text: the row legend, the platform's body, then the
/// screenshot footer with the pixel/point scale. Long bodies are truncated —
/// the picture carries the rest.
pub(crate) fn look_text(shot: &Shot, cropped: bool, body: &str) -> String {
    let mut text = "Rows are @eN name x y. Click that @eN. If a row is not listed, call screen_scroll on the list.\n"
        .to_string();
    if cropped {
        text.push_str("The picture is this app's window.\n");
    }
    text.push_str(body);
    if text.chars().count() > 12000 {
        text.truncate(12000);
        text.push_str("\n…");
    }
    text.push_str(&format!(
        "\n\nScreenshot file: {}\nImage pixels: {}x{}. Scale: {}.\nClick x,y are points ({}x{}), not image pixels. Divide a pixel position by the scale.",
        shot.path, shot.pixels.0, shot.pixels.1, shot.scale, shot.points.0, shot.points.1
    ));
    text
}

/// The `screen_look` reply: the text plus where the picture lives.
pub(crate) fn look_value(text: String, shot: &Shot) -> Value {
    json!({
        "text": text,
        "image": shot.path,
        "pixel_width": shot.pixels.0,
        "pixel_height": shot.pixels.1,
        "scale": shot.scale,
        "point_width": shot.points.0,
        "point_height": shot.points.1,
    })
}
