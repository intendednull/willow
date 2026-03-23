//! Shared text editing helpers for cursor-based input fields.

/// Convert a character index to a byte index in a string.
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Number of characters in a string.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Insert a single character at the cursor position.
pub fn insert_char(text: &mut String, cursor: &mut usize, c: char) {
    let byte = char_to_byte(text, *cursor);
    text.insert(byte, c);
    *cursor += 1;
}

/// Insert a string at the cursor position.
pub fn insert_str(text: &mut String, cursor: &mut usize, s: &str) {
    let byte = char_to_byte(text, *cursor);
    text.insert_str(byte, s);
    *cursor += s.chars().count();
}

/// Delete the character before the cursor (Backspace).
pub fn backspace(text: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        *cursor -= 1;
        let byte = char_to_byte(text, *cursor);
        text.remove(byte);
    }
}

/// Delete the character after the cursor (Delete key).
pub fn delete(text: &mut String, cursor: &mut usize) {
    if *cursor < char_len(text) {
        let byte = char_to_byte(text, *cursor);
        text.remove(byte);
    }
}

/// Move cursor left one character.
pub fn move_left(cursor: &mut usize) {
    *cursor = cursor.saturating_sub(1);
}

/// Move cursor right one character.
pub fn move_right(text: &str, cursor: &mut usize) {
    let len = char_len(text);
    if *cursor < len {
        *cursor += 1;
    }
}

/// Move cursor to start of text (Home).
pub fn move_home(cursor: &mut usize) {
    *cursor = 0;
}

/// Move cursor to end of text (End).
pub fn move_end(text: &str, cursor: &mut usize) {
    *cursor = char_len(text);
}

/// Delete the word before the cursor (Ctrl+Backspace).
pub fn backspace_word(text: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut pos = *cursor;
    // Skip whitespace
    while pos > 0 && chars[pos - 1].is_whitespace() {
        pos -= 1;
    }
    // Skip word chars
    while pos > 0 && !chars[pos - 1].is_whitespace() {
        pos -= 1;
    }
    let start_byte = char_to_byte(text, pos);
    let end_byte = char_to_byte(text, *cursor);
    text.drain(start_byte..end_byte);
    *cursor = pos;
}

/// Delete the word after the cursor (Ctrl+Delete).
pub fn delete_word(text: &mut String, cursor: &mut usize) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if *cursor >= len {
        return;
    }
    let mut pos = *cursor;
    // Skip word chars
    while pos < len && !chars[pos].is_whitespace() {
        pos += 1;
    }
    // Skip whitespace
    while pos < len && chars[pos].is_whitespace() {
        pos += 1;
    }
    let start_byte = char_to_byte(text, *cursor);
    let end_byte = char_to_byte(text, pos);
    text.drain(start_byte..end_byte);
    // cursor stays the same
}

/// Move cursor to the previous word boundary (Ctrl+Left).
pub fn move_word_left(text: &str, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut pos = *cursor;
    // Skip whitespace
    while pos > 0 && chars[pos - 1].is_whitespace() {
        pos -= 1;
    }
    // Skip word chars
    while pos > 0 && !chars[pos - 1].is_whitespace() {
        pos -= 1;
    }
    *cursor = pos;
}

/// Move cursor to the next word boundary (Ctrl+Right).
pub fn move_word_right(text: &str, cursor: &mut usize) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if *cursor >= len {
        return;
    }
    let mut pos = *cursor;
    // Skip word chars
    while pos < len && !chars[pos].is_whitespace() {
        pos += 1;
    }
    // Skip whitespace
    while pos < len && chars[pos].is_whitespace() {
        pos += 1;
    }
    *cursor = pos;
}

/// Split text at cursor position for display.
pub fn split_at_cursor(text: &str, cursor: usize) -> (&str, &str) {
    let byte = char_to_byte(text, cursor);
    (&text[..byte], &text[byte..])
}

/// Clear all text and reset cursor.
pub fn clear(text: &mut String, cursor: &mut usize) {
    text.clear();
    *cursor = 0;
}

// ───── Selection ────────────────────────────────────────────────────────────

/// Get the ordered (start, end) range of a selection.
pub fn selection_range(cursor: usize, selection: usize) -> (usize, usize) {
    if cursor < selection {
        (cursor, selection)
    } else {
        (selection, cursor)
    }
}

/// Delete the selected text, returning true if there was a selection.
pub fn delete_selection(
    text: &mut String,
    cursor: &mut usize,
    selection: &mut Option<usize>,
) -> bool {
    let Some(sel) = selection.take() else {
        return false;
    };
    let (start, end) = selection_range(*cursor, sel);
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    text.drain(start_byte..end_byte);
    *cursor = start;
    true
}

/// Get the selected text as a string slice.
pub fn selected_text(text: &str, cursor: usize, selection: usize) -> &str {
    let (start, end) = selection_range(cursor, selection);
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    &text[start_byte..end_byte]
}

/// Select all text: set selection anchor to 0, cursor to end.
pub fn select_all(text: &str, cursor: &mut usize, selection: &mut Option<usize>) {
    *selection = Some(0);
    *cursor = char_len(text);
}

/// Split text into three parts for display: before selection, selected, after selection.
pub fn split_with_selection(text: &str, cursor: usize, selection: usize) -> (&str, &str, &str) {
    let (start, end) = selection_range(cursor, selection);
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    (
        &text[..start_byte],
        &text[start_byte..end_byte],
        &text[end_byte..],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_char_at_start() {
        let mut text = String::from("ello");
        let mut cursor = 0;
        insert_char(&mut text, &mut cursor, 'h');
        assert_eq!(text, "hello");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn insert_char_in_middle() {
        let mut text = String::from("hllo");
        let mut cursor = 1;
        insert_char(&mut text, &mut cursor, 'e');
        assert_eq!(text, "hello");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn backspace_at_start_does_nothing() {
        let mut text = String::from("hello");
        let mut cursor = 0;
        backspace(&mut text, &mut cursor);
        assert_eq!(text, "hello");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn backspace_removes_before_cursor() {
        let mut text = String::from("hello");
        let mut cursor = 3;
        backspace(&mut text, &mut cursor);
        assert_eq!(text, "helo");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn delete_removes_after_cursor() {
        let mut text = String::from("hello");
        let mut cursor = 2;
        delete(&mut text, &mut cursor);
        assert_eq!(text, "helo");
        assert_eq!(cursor, 2);

        // Delete at end does nothing.
        let mut cursor_end = 4;
        delete(&mut text, &mut cursor_end);
        assert_eq!(text, "helo");
    }

    #[test]
    fn word_boundaries() {
        let text = "hello world foo";
        // Positions: h(0) e(1) l(2) l(3) o(4) ' '(5) w(6) o(7) r(8) l(9) d(10) ' '(11) f(12) o(13) o(14)

        let mut cursor = 12; // at 'f'
        move_word_left(text, &mut cursor);
        assert_eq!(cursor, 6); // start of "world"

        move_word_left(text, &mut cursor);
        assert_eq!(cursor, 0); // start of "hello"

        move_word_right(text, &mut cursor);
        assert_eq!(cursor, 6); // past "hello " -> start of "world"

        move_word_right(text, &mut cursor);
        assert_eq!(cursor, 12); // past "world " -> start of "foo"
    }

    #[test]
    fn insert_str_at_cursor() {
        let mut text = String::from("hd");
        let mut cursor = 1;
        insert_str(&mut text, &mut cursor, "ello worl");
        assert_eq!(text, "hello world");
        assert_eq!(cursor, 10);
    }

    #[test]
    fn unicode_handling() {
        let mut text = String::from("cafe");
        let mut cursor = 4;
        // Insert a multi-byte character.
        insert_char(&mut text, &mut cursor, '\u{0301}'); // combining accent
        assert_eq!(cursor, 5);

        // Test with emoji.
        let mut emoji_text = String::from("hi");
        let mut emoji_cursor = 2;
        insert_str(&mut emoji_text, &mut emoji_cursor, " \u{1F600}!");
        assert_eq!(emoji_text, "hi \u{1F600}!");
        assert_eq!(emoji_cursor, 5); // 'h', 'i', ' ', emoji, '!'

        // Backspace removes the emoji.
        emoji_cursor = 4; // after the emoji
        backspace(&mut emoji_text, &mut emoji_cursor);
        assert_eq!(emoji_text, "hi !");
        assert_eq!(emoji_cursor, 3);
    }

    #[test]
    fn backspace_word_and_delete_word() {
        // backspace_word: from start of "foo", delete "world " backwards.
        let mut text = String::from("hello world foo");
        let mut cursor = 12; // at 'f' in "foo"
        backspace_word(&mut text, &mut cursor);
        assert_eq!(text, "hello foo");
        assert_eq!(cursor, 6);

        // delete_word: from start of "world", delete "world " forwards.
        let mut text2 = String::from("hello world foo");
        let mut cursor2 = 6; // at 'w'
        delete_word(&mut text2, &mut cursor2);
        assert_eq!(text2, "hello foo");
        assert_eq!(cursor2, 6);
    }

    #[test]
    fn split_at_cursor_works() {
        let text = "hello world";
        let (before, after) = split_at_cursor(text, 5);
        assert_eq!(before, "hello");
        assert_eq!(after, " world");
    }

    #[test]
    fn clear_resets_both() {
        let mut text = String::from("hello");
        let mut cursor = 3;
        clear(&mut text, &mut cursor);
        assert_eq!(text, "");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn move_home_and_end() {
        let text = "hello";
        let mut cursor = 3;
        move_home(&mut cursor);
        assert_eq!(cursor, 0);
        move_end(text, &mut cursor);
        assert_eq!(cursor, 5);
    }

    #[test]
    fn move_left_right_bounds() {
        let text = "hi";
        let mut cursor = 0;
        move_left(&mut cursor);
        assert_eq!(cursor, 0); // can't go left from 0

        move_right(text, &mut cursor);
        assert_eq!(cursor, 1);
        move_right(text, &mut cursor);
        assert_eq!(cursor, 2);
        move_right(text, &mut cursor);
        assert_eq!(cursor, 2); // can't go past end
    }
}
