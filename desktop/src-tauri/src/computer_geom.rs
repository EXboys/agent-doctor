//! Point math and key names shared by the Windows computer hand.
//!
//! The screenshot is physical pixels. The model clicks in points: pixels divided
//! by `scale` (DPI / 96). `SendInput` then maps those pixels onto the virtual screen.

/// Screenshot the crop is taken from. Coordinates are points unless `scale` is 1.
#[derive(Clone, Copy)]
pub(crate) struct Picture {
    pub origin_x: f64,
    pub origin_y: f64,
    pub scale: f64,
    pub pixel_w: i64,
    pub pixel_h: i64,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyPress {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub vk: u16,
    /// Dedicated Delete, arrows, Home, and End. Without this, Windows treats
    /// Delete as the numpad key and many text fields ignore it.
    pub extended: bool,
}

#[cfg(any(windows, test))]
pub(crate) fn scale_of(dpi: i32) -> f64 {
    if dpi <= 96 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

#[cfg(any(windows, test))]
pub(crate) fn point_of(pixel: i32, origin: i32, scale: f64) -> i64 {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    ((pixel - origin) as f64 / scale).round() as i64
}

#[cfg(any(windows, test))]
pub(crate) fn pixel_of(point: f64, origin: i32, scale: f64) -> i32 {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    origin.saturating_add((point * scale).round() as i32)
}

/// `SendInput` absolute axis: 0 at the virtual-screen origin, 65535 at the far edge.
#[cfg(any(windows, test))]
pub(crate) fn absolute_axis(pixel: i32, origin: i32, span: i32) -> i32 {
    if span <= 1 {
        return 0;
    }
    let delta = pixel.saturating_sub(origin).clamp(0, span - 1);
    ((delta as i64) * 65535 / (span as i64 - 1)) as i32
}

pub(crate) fn window_has_control(text: &str) -> bool {
    text.lines().any(|line| {
        matches!(
            line.split('\t').next().unwrap_or("").trim(),
            "button" | "text field" | "text"
        )
    })
}

pub(crate) fn window_has_field(text: &str) -> bool {
    text.lines()
        .any(|line| line.split('\t').next().unwrap_or("").trim() == "text field")
}

pub(crate) fn point_inside(x: f64, y: f64, width: f64, height: f64) -> bool {
    width > 0.0 && height > 0.0 && x >= 0.0 && y >= 0.0 && x < width && y < height
}

/// A clock tick or a one-pixel shift is not a new screen. A control that
/// moved or a window that opened is.
pub(crate) fn picture_changed(before: &[u8], after: &[u8]) -> bool {
    if before.len() != after.len() || before.is_empty() {
        return true;
    }
    let changed = before
        .iter()
        .zip(after)
        .filter(|(left, right)| left.abs_diff(**right) > 28)
        .count();
    changed >= 4
}

/// Pixel box of a window inside a screenshot. None when it misses the picture
/// or already fills it. `scale` turns window points into pixels; pass `1.0`
/// when the window rect is already in pixels.
pub(crate) fn crop_pixels(
    picture: Picture,
    win_x: f64,
    win_y: f64,
    win_w: f64,
    win_h: f64,
) -> Option<(u32, u32, u32, u32)> {
    if picture.scale <= 0.0
        || picture.pixel_w < 40
        || picture.pixel_h < 40
        || win_w < 40.0
        || win_h < 40.0
    {
        return None;
    }
    let left = ((win_x - picture.origin_x) * picture.scale)
        .floor()
        .max(0.0);
    let top = ((win_y - picture.origin_y) * picture.scale)
        .floor()
        .max(0.0);
    let right = ((win_x + win_w - picture.origin_x) * picture.scale)
        .ceil()
        .min(picture.pixel_w as f64);
    let bottom = ((win_y + win_h - picture.origin_y) * picture.scale)
        .ceil()
        .min(picture.pixel_h as f64);
    let width = right - left;
    let height = bottom - top;
    if width < 40.0 || height < 40.0 {
        return None;
    }
    let shot_area = picture.pixel_w * picture.pixel_h;
    let area = (width as i64) * (height as i64);
    if shot_area > 0 && area * 100 / shot_area >= 92 {
        return None;
    }
    Some((left as u32, top as u32, width as u32, height as u32))
}

#[cfg(any(windows, test))]
pub(crate) fn control_line(role: &str, label: &str, x: i64, y: i64) -> String {
    let label = label.replace(['\t', '\n', '\r'], " ");
    format!("{role}\t{label}\t{x}\t{y}\n")
}

/// `command` is Ctrl here. The tool description says `command+a`, and on Windows that selects all.
#[cfg(any(windows, test))]
pub(crate) fn parse_key(key: &str) -> Result<KeyPress, String> {
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        return Err("key is empty".into());
    }
    let mut press = KeyPress {
        control: false,
        shift: false,
        alt: false,
        win: false,
        vk: 0,
        extended: false,
    };
    let mut token = String::new();
    for part in key.split('+') {
        match part.trim() {
            "command" | "cmd" | "meta" | "control" | "ctrl" => press.control = true,
            "shift" => press.shift = true,
            "option" | "alt" => press.alt = true,
            "win" | "windows" => press.win = true,
            other if !other.is_empty() => token = other.to_string(),
            _ => {}
        }
    }
    let (vk, extended) = match token.as_str() {
        "return" | "enter" => (0x0D, false),
        "escape" | "esc" => (0x1B, false),
        "tab" => (0x09, false),
        "space" => (0x20, false),
        // The character before the cursor. This is the key people use to edit a field.
        "backspace" | "bksp" | "bs" | "退格" => (0x08, false),
        // The character after the cursor. Must be the extended Delete key.
        "delete" | "del" | "删除" => (0x2E, true),
        "insert" | "ins" => (0x2D, true),
        "home" => (0x24, true),
        "end" => (0x23, true),
        "pageup" | "page_up" | "pgup" | "page up" => (0x21, true),
        "pagedown" | "page_down" | "pgdn" | "page down" => (0x22, true),
        "up" => (0x26, true),
        "down" => (0x28, true),
        "left" => (0x25, true),
        "right" => (0x27, true),
        other if other.len() == 1 && other.as_bytes()[0].is_ascii_digit() => {
            (other.as_bytes()[0] as u16, false)
        }
        other if other.len() == 1 && other.as_bytes()[0].is_ascii_lowercase() => {
            (other.as_bytes()[0].to_ascii_uppercase() as u16, false)
        }
        other => match function_vk(other) {
            Some(vk) => (vk, false),
            None => {
                return Err(format!(
                    "unknown key: {other}. Use backspace to remove the character before the cursor, or delete for the one after it."
                ));
            }
        },
    };
    press.vk = vk;
    press.extended = extended;
    Ok(press)
}

/// `f1`…`f12`. `VK_F1` is `0x70`.
#[cfg(any(windows, test))]
fn function_vk(token: &str) -> Option<u16> {
    let rest = token.strip_prefix('f')?;
    let n: u16 = rest.parse().ok()?;
    if (1..=12).contains(&n) {
        Some(0x6F + n)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        absolute_axis, control_line, crop_pixels, parse_key, picture_changed, pixel_of,
        point_inside, point_of, scale_of, window_has_control, window_has_field,
    };

    #[test]
    fn dpi_becomes_the_point_scale() {
        assert_eq!(scale_of(96), 1.0);
        assert_eq!(scale_of(0), 1.0);
        assert!((scale_of(144) - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn points_round_trip_through_pixels() {
        let scale = 1.5;
        assert_eq!(point_of(150, 0, scale), 100);
        assert_eq!(pixel_of(100.0, 0, scale), 150);
        assert_eq!(point_of(150, -100, scale), 167);
        assert_eq!(pixel_of(100.0, -100, scale), 50);
    }

    #[test]
    fn absolute_axis_spans_the_virtual_screen() {
        assert_eq!(absolute_axis(0, 0, 1920), 0);
        assert_eq!(absolute_axis(1919, 0, 1920), 65535);
        assert_eq!(absolute_axis(-50, 0, 1920), 0);
        assert_eq!(absolute_axis(10, 10, 100), 0);
        // A monitor sitting left of the primary still maps through the whole virtual screen.
        let pixel = pixel_of(100.0, -1920, 1.0);
        assert_eq!(pixel, -1820);
        assert_eq!(absolute_axis(pixel, -1920, 3840), (100 * 65535) / 3839);
    }

    #[test]
    fn command_selects_with_ctrl() {
        let key = parse_key("command+a").unwrap();
        assert!(key.control);
        assert_eq!(key.vk, b'A' as u16);
        let enter = parse_key("ctrl+enter").unwrap();
        assert!(enter.control);
        assert_eq!(enter.vk, 0x0D);
    }

    #[test]
    fn delete_and_backspace_reach_a_text_field() {
        let back = parse_key("backspace").unwrap();
        assert_eq!(back.vk, 0x08);
        assert!(!back.extended);
        let del = parse_key("delete").unwrap();
        assert_eq!(del.vk, 0x2E);
        assert!(del.extended);
        let chord = parse_key("ctrl+退格").unwrap();
        assert!(chord.control);
        assert_eq!(chord.vk, 0x08);
        assert_eq!(parse_key("删除").unwrap().vk, 0x2E);
        assert!(parse_key("up").unwrap().extended);
        assert_eq!(parse_key("f5").unwrap().vk, 0x74);
        assert!(parse_key("not-a-key").is_err());
    }

    #[test]
    fn control_lines_keep_four_columns() {
        let line = control_line("button", "Save\tme", 12, 34);
        assert_eq!(line, "button\tSave me\t12\t34\n");
    }

    #[test]
    fn a_drawn_window_has_no_named_control() {
        let text = "front app: 微信\nwindow: 微信\n";
        assert!(!window_has_control(text));
        assert!(!window_has_field(text));
        let field = "text field\t搜索\t12\t40\n";
        assert!(window_has_control(field));
        assert!(window_has_field(field));
    }

    #[test]
    fn a_point_outside_the_picture_is_rejected() {
        assert!(point_inside(10.0, 10.0, 100.0, 80.0));
        assert!(!point_inside(100.0, 10.0, 100.0, 80.0));
        assert!(!point_inside(-1.0, 10.0, 100.0, 80.0));
        assert!(!point_inside(1.0, 1.0, 0.0, 80.0));
    }

    #[test]
    fn a_still_picture_is_not_a_change() {
        let still = vec![10u8; 48 * 48];
        assert!(!picture_changed(&still, &still));
        let mut flicker = still.clone();
        flicker[0] = 255;
        assert!(!picture_changed(&still, &flicker));
        let mut moved = still.clone();
        for pixel in moved.iter_mut().take(8) {
            *pixel = 255;
        }
        assert!(picture_changed(&still, &moved));
        assert!(picture_changed(&[], &still));
    }

    #[test]
    fn a_window_crop_stays_inside_the_picture() {
        let picture = super::Picture {
            origin_x: 0.0,
            origin_y: 0.0,
            scale: 2.0,
            pixel_w: 2000,
            pixel_h: 1200,
        };
        let crop = crop_pixels(picture, 100.0, 80.0, 400.0, 300.0).unwrap();
        assert_eq!(crop, (200, 160, 800, 600));
        assert!(crop_pixels(picture, 0.0, 0.0, 1000.0, 600.0).is_none());
        assert!(crop_pixels(picture, 4000.0, 80.0, 100.0, 100.0).is_none());
    }
}
