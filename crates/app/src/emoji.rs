//! # Emoji
//!
//! Shortcode expansion for standard Unicode emoji and server-defined custom
//! emoji. Shortcodes use the `:name:` syntax (e.g., `:thumbsup:` → `👍`).

use std::collections::HashMap;

/// Registry mapping `:shortcode:` names → replacement strings.
///
/// Includes built-in Unicode mappings and server-defined custom entries.
#[derive(Debug, Clone, Default)]
pub struct EmojiRegistry {
    custom: HashMap<String, String>,
}

impl EmojiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a custom emoji shortcode.
    pub fn add(&mut self, shortcode: impl Into<String>, value: impl Into<String>) {
        self.custom.insert(shortcode.into(), value.into());
    }

    /// Remove a custom emoji.
    pub fn remove(&mut self, shortcode: &str) {
        self.custom.remove(shortcode);
    }

    /// List all custom emoji (not built-ins).
    pub fn custom_entries(&self) -> &HashMap<String, String> {
        &self.custom
    }

    /// Look up a shortcode, checking custom entries first, then built-ins.
    pub fn get(&self, shortcode: &str) -> Option<&str> {
        self.custom
            .get(shortcode)
            .map(|s| s.as_str())
            .or_else(|| builtin(shortcode))
    }

    /// Expand all `:shortcode:` patterns in a string.
    pub fn expand(&self, text: &str) -> String {
        expand_shortcodes(text, |code| self.get(code).map(|s| s.to_string()))
    }
}

/// Expand `:shortcode:` patterns using a lookup function.
fn expand_shortcodes(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c == ':' {
            // Look for closing `:`.
            let _start = i;
            let code_start = i + 1;
            let mut found_end = false;

            // Collect characters until we find another `:` or a space/newline.
            let mut end = code_start;
            for (j, c2) in text[code_start..].char_indices() {
                if c2 == ':' {
                    end = code_start + j;
                    found_end = true;
                    break;
                }
                if c2.is_whitespace() || c2 == '\n' {
                    break;
                }
            }

            if found_end && end > code_start {
                let code = &text[code_start..end];
                if let Some(replacement) = lookup(code) {
                    result.push_str(&replacement);
                    // Advance past the closing `:`.
                    while let Some((j, _)) = chars.peek() {
                        if *j <= end {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    continue;
                }
            }

            // Not a valid shortcode, emit the `:` literally.
            result.push(c);
        } else {
            result.push(c);
        }
    }

    result
}

/// Built-in emoji shortcode lookup.
fn builtin(code: &str) -> Option<&'static str> {
    Some(match code {
        // Smileys
        "smile" | "smiley" => "😄",
        "grin" => "😁",
        "laugh" | "joy" => "😂",
        "rofl" => "🤣",
        "wink" => "😉",
        "blush" => "😊",
        "heart_eyes" => "😍",
        "kissing" => "😘",
        "thinking" => "🤔",
        "shush" | "shushing" => "🤫",
        "sweat" => "😅",
        "cry" | "sob" => "😭",
        "scream" => "😱",
        "angry" => "😠",
        "rage" => "🤬",
        "skull" => "💀",
        "clown" => "🤡",
        "eyes" => "👀",
        "brain" => "🧠",
        "nerd" => "🤓",
        "cool" | "sunglasses" => "😎",
        "sleeping" | "zzz" => "😴",
        "drool" => "🤤",
        "shrug" => "🤷",

        // Gestures
        "thumbsup" | "+1" | "thumbs_up" => "👍",
        "thumbsdown" | "-1" | "thumbs_down" => "👎",
        "wave" => "👋",
        "clap" => "👏",
        "handshake" => "🤝",
        "pray" | "folded_hands" => "🙏",
        "muscle" | "flex" => "💪",
        "point_up" => "☝️",
        "point_right" => "👉",
        "point_left" => "👈",
        "point_down" => "👇",
        "ok_hand" | "ok" => "👌",
        "v" | "peace" => "✌️",
        "crossed_fingers" => "🤞",
        "metal" | "rock" => "🤘",
        "raised_hands" | "hooray" => "🙌",
        "fist" => "✊",
        "fire" | "lit" => "🔥",

        // Hearts
        "heart" | "love" => "❤️",
        "orange_heart" => "🧡",
        "yellow_heart" => "💛",
        "green_heart" => "💚",
        "blue_heart" => "💙",
        "purple_heart" => "💜",
        "broken_heart" => "💔",
        "sparkling_heart" => "💖",
        "heartbeat" => "💓",

        // Objects
        "star" => "⭐",
        "sparkles" => "✨",
        "tada" | "party" => "🎉",
        "balloon" => "🎈",
        "gift" => "🎁",
        "trophy" => "🏆",
        "medal" => "🏅",
        "crown" => "👑",
        "gem" | "diamond" => "💎",
        "money" | "dollar" => "💵",
        "bulb" | "idea" | "lightbulb" => "💡",
        "lock" => "🔒",
        "unlock" => "🔓",
        "key" => "🔑",
        "hammer" => "🔨",
        "wrench" => "🔧",
        "gear" | "settings" => "⚙️",
        "link" => "🔗",
        "pin" | "pushpin" => "📌",
        "bell" => "🔔",
        "mega" | "megaphone" => "📣",

        // Animals & Nature
        "dog" => "🐕",
        "cat" => "🐈",
        "tree" => "🌳",
        "willow" => "🌿",
        "flower" | "blossom" => "🌸",
        "sun" | "sunny" => "☀️",
        "moon" => "🌙",
        "earth" | "globe" | "world" => "🌍",
        "rainbow" => "🌈",
        "snowflake" => "❄️",
        "lightning" | "zap" => "⚡",
        "umbrella" => "☂️",

        // Food
        "coffee" => "☕",
        "pizza" => "🍕",
        "beer" => "🍺",
        "wine" => "🍷",
        "cake" => "🎂",
        "cookie" => "🍪",
        "taco" => "🌮",

        // Tech
        "computer" | "laptop" => "💻",
        "phone" | "mobile" => "📱",
        "keyboard" => "⌨️",
        "bug" => "🐛",
        "robot" => "🤖",
        "rocket" => "🚀",
        "satellite" => "🛰️",

        // Symbols
        "check" | "white_check_mark" => "✅",
        "x" | "cross" => "❌",
        "warning" => "⚠️",
        "question" => "❓",
        "exclamation" | "bang" => "❗",
        "100" | "hundred" => "💯",
        "plus" => "➕",
        "minus" => "➖",
        "arrow_right" => "➡️",
        "arrow_left" => "⬅️",
        "arrow_up" => "⬆️",
        "arrow_down" => "⬇️",
        "recycle" => "♻️",
        "infinity" => "♾️",

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lookup() {
        assert_eq!(builtin("thumbsup"), Some("👍"));
        assert_eq!(builtin("heart"), Some("❤️"));
        assert_eq!(builtin("fire"), Some("🔥"));
        assert_eq!(builtin("nonexistent"), None);
    }

    #[test]
    fn expand_single_shortcode() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":thumbsup:"), "👍");
    }

    #[test]
    fn expand_multiple_shortcodes() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":fire: :rocket:"), "🔥 🚀");
    }

    #[test]
    fn expand_mixed_text() {
        let reg = EmojiRegistry::new();
        assert_eq!(
            reg.expand("great job :thumbsup: keep going :fire:"),
            "great job 👍 keep going 🔥"
        );
    }

    #[test]
    fn expand_unknown_shortcode_preserved() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":unknown:"), ":unknown:");
    }

    #[test]
    fn expand_no_shortcodes() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand("hello world"), "hello world");
    }

    #[test]
    fn expand_incomplete_shortcode() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand("time is 3:00"), "time is 3:00");
    }

    #[test]
    fn expand_colon_in_middle_of_word() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand("http://example.com"), "http://example.com");
    }

    #[test]
    fn custom_emoji_override() {
        let mut reg = EmojiRegistry::new();
        reg.add("willow", "🌲");
        assert_eq!(reg.expand(":willow:"), "🌲");
    }

    #[test]
    fn custom_emoji_text_value() {
        let mut reg = EmojiRegistry::new();
        reg.add("shrug_text", r"¯\_(ツ)_/¯");
        assert_eq!(reg.expand(":shrug_text:"), r"¯\_(ツ)_/¯");
    }

    #[test]
    fn custom_overrides_builtin() {
        let mut reg = EmojiRegistry::new();
        reg.add("fire", "🧊"); // override fire with ice
        assert_eq!(reg.expand(":fire:"), "🧊");
    }

    #[test]
    fn remove_custom_emoji() {
        let mut reg = EmojiRegistry::new();
        reg.add("test", "TEST");
        assert_eq!(reg.get("test"), Some("TEST"));
        reg.remove("test");
        assert_eq!(reg.get("test"), None);
    }

    #[test]
    fn empty_shortcode_not_expanded() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand("::"), "::");
    }

    #[test]
    fn adjacent_shortcodes() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":fire::fire:"), "🔥🔥");
    }
}
