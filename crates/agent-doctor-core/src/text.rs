//! Small string helpers for text that gets cut for display.
//!
//! Slicing a `str` by a byte index panics when the index lands inside a
//! multi-byte character. Chinese text is common in this app (model names,
//! error lines, masked secrets), so every byte-indexed cut goes through the
//! helpers here instead of panicking on the reader thread.
//!
//! `str::floor_char_boundary` only stabilised in Rust 1.91 and this project
//! still supports 1.74, so the boundary math lives here.

/// Largest byte index `<= idx` that is a character boundary of `text`.
///
/// `idx` is clamped to the end of `text`.
pub fn floor_char_boundary(text: &str, idx: usize) -> usize {
    let mut cut = idx.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Smallest byte index `>= idx` that is a character boundary of `text`.
///
/// `idx` is clamped to the end of `text`.
pub fn ceil_char_boundary(text: &str, idx: usize) -> usize {
    let mut cut = idx.min(text.len());
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    cut
}

/// `text` cut to at most `max_bytes`, never splitting a character.
pub fn clip_bytes(text: &str, max_bytes: usize) -> &str {
    &text[..floor_char_boundary(text, max_bytes)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_inside_a_multi_byte_character_without_panicking() {
        // Every character is three bytes, so byte 5 is inside the second one.
        let text = "中文测试";
        assert_eq!(clip_bytes(text, 5), "中");
        assert_eq!(clip_bytes(text, 6), "中文");
        assert_eq!(clip_bytes(text, 100), text);
        assert_eq!(clip_bytes(text, 0), "");
    }

    #[test]
    fn rounds_a_byte_index_to_the_nearest_boundary() {
        let text = "a中b";
        assert_eq!(floor_char_boundary(text, 3), 1);
        assert_eq!(ceil_char_boundary(text, 3), 4);
        assert_eq!(floor_char_boundary(text, 0), 0);
        assert_eq!(ceil_char_boundary(text, 5), 5);
    }
}
