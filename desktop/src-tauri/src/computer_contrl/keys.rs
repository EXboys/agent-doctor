//! Press one key, or a chord such as command+v.
//!
//! Reference: trycua/cua `platform-macos` `press_key` and `hotkey`.

/// A parsed key. Modifiers live in `flags`. One key has no extra flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub flags: u64,
    pub code: u16,
}

pub fn parse(spec: &str) -> Result<Chord, String> {
    let spec = spec.trim().to_ascii_lowercase();
    if spec.is_empty() {
        return Err("key is empty".into());
    }
    let mut flags = 0u64;
    let mut token = String::new();
    for part in spec.split('+') {
        match part.trim() {
            "command" | "cmd" | "meta" => flags |= 0x0010_0000,
            "shift" => flags |= 0x0002_0000,
            "option" | "alt" => flags |= 0x0008_0000,
            "control" | "ctrl" => flags |= 0x0004_0000,
            other if !other.is_empty() => token = other.to_string(),
            _ => {}
        }
    }
    let code = match token.as_str() {
        "return" | "enter" => 36,
        "escape" | "esc" => 53,
        "tab" => 48,
        "space" => 49,
        "up" => 126,
        "down" => 125,
        "left" => 123,
        "right" => 124,
        "a" => 0,
        "b" => 11,
        "c" => 8,
        "d" => 2,
        "e" => 14,
        "f" => 3,
        "g" => 5,
        "h" => 4,
        "i" => 34,
        "j" => 38,
        "k" => 40,
        "l" => 37,
        "m" => 46,
        "n" => 45,
        "o" => 31,
        "p" => 35,
        "q" => 12,
        "r" => 15,
        "s" => 1,
        "t" => 17,
        "u" => 32,
        "v" => 9,
        "w" => 13,
        "x" => 7,
        "y" => 16,
        "z" => 6,
        other => return Err(format!("unknown key: {other}")),
    };
    Ok(Chord { flags, code })
}

pub fn press(spec: &str) -> Result<(), String> {
    let chord = parse(spec)?;
    #[cfg(target_os = "macos")]
    {
        super::mac::post_key(chord.code, chord.flags)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = chord;
        Err("目前只在 Mac 上。".into())
    }
}

pub fn hotkey(spec: &str) -> Result<(), String> {
    let spec = spec.trim();
    if !spec.contains('+') {
        return Err("组合键要写成 command+v 这样。".into());
    }
    press(spec)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_chord_keeps_the_modifier_and_the_key() {
        let chord = parse("command+c").unwrap();
        assert_eq!(chord.flags, 0x0010_0000);
        assert_eq!(chord.code, 8);
        assert_eq!(parse("return").unwrap().code, 36);
        assert!(parse("").is_err());
        assert!(parse("command+").is_err());
    }
}
