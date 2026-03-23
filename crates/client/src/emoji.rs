//! # Emoji
//!
//! Shortcode expansion for standard Unicode emoji and server-defined custom
//! emoji. Shortcodes use the `:name:` syntax (e.g., `:thumbsup:` -> `thumbsup_emoji`).

use std::collections::HashMap;

/// Registry mapping `:shortcode:` names -> replacement strings.
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
        "smile" | "smiley" => "\u{1f604}",
        "grin" => "\u{1f601}",
        "laugh" | "joy" => "\u{1f602}",
        "rofl" => "\u{1f923}",
        "wink" => "\u{1f609}",
        "blush" => "\u{1f60a}",
        "heart_eyes" => "\u{1f60d}",
        "kissing" => "\u{1f618}",
        "thinking" => "\u{1f914}",
        "shush" | "shushing" => "\u{1f92b}",
        "sweat" => "\u{1f605}",
        "cry" | "sob" => "\u{1f62d}",
        "scream" => "\u{1f631}",
        "angry" => "\u{1f620}",
        "rage" => "\u{1f92c}",
        "skull" => "\u{1f480}",
        "clown" => "\u{1f921}",
        "eyes" => "\u{1f440}",
        "brain" => "\u{1f9e0}",
        "nerd" => "\u{1f913}",
        "cool" | "sunglasses" => "\u{1f60e}",
        "sleeping" | "zzz" => "\u{1f634}",
        "drool" => "\u{1f924}",
        "shrug" => "\u{1f937}",

        // Gestures
        "thumbsup" | "+1" | "thumbs_up" => "\u{1f44d}",
        "thumbsdown" | "-1" | "thumbs_down" => "\u{1f44e}",
        "wave" => "\u{1f44b}",
        "clap" => "\u{1f44f}",
        "handshake" => "\u{1f91d}",
        "pray" | "folded_hands" => "\u{1f64f}",
        "muscle" | "flex" => "\u{1f4aa}",
        "point_up" => "\u{261d}\u{fe0f}",
        "point_right" => "\u{1f449}",
        "point_left" => "\u{1f448}",
        "point_down" => "\u{1f447}",
        "ok_hand" | "ok" => "\u{1f44c}",
        "v" | "peace" => "\u{270c}\u{fe0f}",
        "crossed_fingers" => "\u{1f91e}",
        "metal" | "rock" => "\u{1f918}",
        "raised_hands" | "hooray" => "\u{1f64c}",
        "fist" => "\u{270a}",
        "fire" | "lit" => "\u{1f525}",

        // Hearts
        "heart" | "love" => "\u{2764}\u{fe0f}",
        "orange_heart" => "\u{1f9e1}",
        "yellow_heart" => "\u{1f49b}",
        "green_heart" => "\u{1f49a}",
        "blue_heart" => "\u{1f499}",
        "purple_heart" => "\u{1f49c}",
        "broken_heart" => "\u{1f494}",
        "sparkling_heart" => "\u{1f496}",
        "heartbeat" => "\u{1f493}",

        // Objects
        "star" => "\u{2b50}",
        "sparkles" => "\u{2728}",
        "tada" | "party" => "\u{1f389}",
        "balloon" => "\u{1f388}",
        "gift" => "\u{1f381}",
        "trophy" => "\u{1f3c6}",
        "medal" => "\u{1f3c5}",
        "crown" => "\u{1f451}",
        "gem" | "diamond" => "\u{1f48e}",
        "money" | "dollar" => "\u{1f4b5}",
        "bulb" | "idea" | "lightbulb" => "\u{1f4a1}",
        "lock" => "\u{1f512}",
        "unlock" => "\u{1f513}",
        "key" => "\u{1f511}",
        "hammer" => "\u{1f528}",
        "wrench" => "\u{1f527}",
        "gear" | "settings" => "\u{2699}\u{fe0f}",
        "link" => "\u{1f517}",
        "pin" | "pushpin" => "\u{1f4cc}",
        "bell" => "\u{1f514}",
        "mega" | "megaphone" => "\u{1f4e3}",

        // Animals & Nature
        "dog" => "\u{1f415}",
        "cat" => "\u{1f408}",
        "tree" => "\u{1f333}",
        "willow" => "\u{1f33f}",
        "flower" | "blossom" => "\u{1f338}",
        "sun" | "sunny" => "\u{2600}\u{fe0f}",
        "moon" => "\u{1f319}",
        "earth" | "globe" | "world" => "\u{1f30d}",
        "rainbow" => "\u{1f308}",
        "snowflake" => "\u{2744}\u{fe0f}",
        "lightning" | "zap" => "\u{26a1}",
        "umbrella" => "\u{2602}\u{fe0f}",

        // Food
        "coffee" => "\u{2615}",
        "pizza" => "\u{1f355}",
        "beer" => "\u{1f37a}",
        "wine" => "\u{1f377}",
        "cake" => "\u{1f382}",
        "cookie" => "\u{1f36a}",
        "taco" => "\u{1f32e}",

        // Tech
        "computer" | "laptop" => "\u{1f4bb}",
        "phone" | "mobile" => "\u{1f4f1}",
        "keyboard" => "\u{2328}\u{fe0f}",
        "bug" => "\u{1f41b}",
        "robot" => "\u{1f916}",
        "rocket" => "\u{1f680}",
        "satellite" => "\u{1f6f0}\u{fe0f}",

        // Symbols
        "check" | "white_check_mark" => "\u{2705}",
        "x" | "cross" => "\u{274c}",
        "warning" => "\u{26a0}\u{fe0f}",
        "question" => "\u{2753}",
        "exclamation" | "bang" => "\u{2757}",
        "100" | "hundred" => "\u{1f4af}",
        "plus" => "\u{2795}",
        "minus" => "\u{2796}",
        "arrow_right" => "\u{27a1}\u{fe0f}",
        "arrow_left" => "\u{2b05}\u{fe0f}",
        "arrow_up" => "\u{2b06}\u{fe0f}",
        "arrow_down" => "\u{2b07}\u{fe0f}",
        "recycle" => "\u{267b}\u{fe0f}",
        "infinity" => "\u{267e}\u{fe0f}",

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lookup() {
        assert_eq!(builtin("thumbsup"), Some("\u{1f44d}"));
        assert_eq!(builtin("heart"), Some("\u{2764}\u{fe0f}"));
        assert_eq!(builtin("fire"), Some("\u{1f525}"));
        assert_eq!(builtin("nonexistent"), None);
    }

    #[test]
    fn expand_single_shortcode() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":thumbsup:"), "\u{1f44d}");
    }

    #[test]
    fn expand_multiple_shortcodes() {
        let reg = EmojiRegistry::new();
        assert_eq!(reg.expand(":fire: :rocket:"), "\u{1f525} \u{1f680}");
    }

    #[test]
    fn expand_mixed_text() {
        let reg = EmojiRegistry::new();
        assert_eq!(
            reg.expand("great job :thumbsup: keep going :fire:"),
            "great job \u{1f44d} keep going \u{1f525}"
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
        reg.add("willow", "\u{1f332}");
        assert_eq!(reg.expand(":willow:"), "\u{1f332}");
    }

    #[test]
    fn custom_emoji_text_value() {
        let mut reg = EmojiRegistry::new();
        reg.add("shrug_text", r"\_(ツ)_/");
        assert_eq!(reg.expand(":shrug_text:"), r"\_(ツ)_/");
    }

    #[test]
    fn custom_overrides_builtin() {
        let mut reg = EmojiRegistry::new();
        reg.add("fire", "\u{1f9ca}"); // override fire with ice
        assert_eq!(reg.expand(":fire:"), "\u{1f9ca}");
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
        assert_eq!(reg.expand(":fire::fire:"), "\u{1f525}\u{1f525}");
    }
}
