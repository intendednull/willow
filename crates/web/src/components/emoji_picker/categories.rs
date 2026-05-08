//! Static emoji category data + the picker's filter helper.
//!
//! The category table is a small inline `&[(&str, &[&str])]` covering
//! the spec's six named categories (smileys / nature / food / travel /
//! objects / symbols). Coverage targets the most-clicked glyphs in a
//! chat context rather than full Unicode parity — the picker is
//! search-first so the long tail surfaces by name. A future expansion
//! can drop in a generated full-table without changing callers.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Emoji picker.

/// One picker category. The first tuple element is the display label
/// (rendered as the section header in the picker). The second is the
/// glyph list, in the order they should appear in the grid.
pub type Category = (&'static str, &'static [&'static str]);

/// Inline static category table.
///
/// Categories follow the spec's order: smileys → nature → food →
/// travel → objects → symbols. A "recent" category is composed at
/// render time from the [`willow_client::ClientHandle::recent_reactions`]
/// LRU and is NOT in this static table — it sits above the static
/// categories with its own header.
pub const EMOJI_CATEGORIES: &[Category] = &[
    (
        "smileys",
        &[
            "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰",
            "😍", "🤩", "😘", "😗", "😚", "😙", "🥲", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗",
            "🤭", "🤫", "🤔", "🤐", "😐", "😑", "😶", "😏", "😒", "🙄", "😬", "😮", "😯", "😲",
            "🥱", "😴", "🤤", "😪", "😵", "🤐", "🥴", "🤢", "🤧", "🤒", "🤕", "🤠", "🥳", "🥸",
            "😎", "🤓", "🧐",
        ],
    ),
    (
        "nature",
        &[
            "🌱", "🌿", "🍀", "🍃", "🌳", "🌲", "🌴", "🌵", "🌾", "🌷", "🌸", "🌹", "🥀", "🌺",
            "🌻", "🌼", "🌽", "🍄", "🌰", "🌍", "🌎", "🌏", "🌑", "🌒", "🌓", "🌔", "🌕", "🌖",
            "🌗", "🌘", "🌙", "🌚", "🌛", "🌜", "🌡", "☀️", "🌝", "🌞", "⭐", "🌟", "🌠", "☁️",
            "⛅", "⛈", "🌤", "🌥", "🌦", "🌧", "🌨", "🌩", "🌪", "🌫", "🌬", "🌈", "❄️", "🔥", "💧", "💦",
            "🌊", "🐢", "🐍", "🦖", "🦕", "🐳", "🐬", "🐠",
        ],
    ),
    (
        "food",
        &[
            "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🫐", "🍈", "🍒", "🍑", "🥭",
            "🍍", "🥥", "🥝", "🍅", "🍆", "🥑", "🥦", "🥬", "🥒", "🌶", "🫑", "🌽", "🥕", "🫒",
            "🧄", "🧅", "🥔", "🍠", "🥐", "🥯", "🍞", "🥖", "🥨", "🧀", "🥚", "🍳", "🧈", "🥞",
            "🧇", "🥓", "🥩", "🍗", "🍖", "🌭", "🍔", "🍟", "🍕", "🥪", "🥙", "🧆", "🌮", "🌯",
            "🥗", "🍲", "🍝", "🍜", "🍣", "🍱", "🥟", "🍦", "🍰", "🧁", "🍫", "🍪", "🍿", "☕",
            "🍵", "🧋", "🥛", "🍺", "🍷", "🥂", "🥃",
        ],
    ),
    (
        "travel",
        &[
            "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐", "🚚", "🚛", "🚜", "🛴",
            "🚲", "🛵", "🏍", "🛺", "🚨", "🚔", "🚍", "🚘", "🚖", "🚡", "🚠", "🚟", "🚃", "🚋",
            "🚞", "🚝", "🚄", "🚅", "🚈", "🚂", "🚆", "🚇", "🚊", "🚉", "✈️", "🛫", "🛬", "🛩",
            "💺", "🛰", "🚀", "🛸", "🚁", "🛶", "⛵", "🚤", "🛥", "🛳", "⛴", "🚢", "⚓", "⛽", "🚧",
            "🚦", "🚥", "🗺", "🗿", "🗽", "🗼", "🏰", "🏯", "🏟", "🎡", "🎢", "🎠", "⛲", "⛱", "🏖",
            "🏝", "🏜", "🌋", "⛰", "🏔", "🗻", "🏕", "⛺", "🏠", "🏡", "🏘", "🏚", "🏗", "🏭", "🏢", "🏬",
            "🏣", "🏤", "🏥", "🏦", "🏨", "🏪", "🏫", "🏩", "💒", "🏛", "⛪", "🕌", "🕍", "🛕",
            "🕋", "⛩",
        ],
    ),
    (
        "objects",
        &[
            "⌚", "📱", "📲", "💻", "⌨️", "🖥", "🖨", "🖱", "🖲", "🕹", "🗜", "💽", "💾", "💿", "📀",
            "📼", "📷", "📸", "📹", "🎥", "📽", "🎞", "📞", "☎️", "📟", "📠", "📺", "📻", "🎙", "🎚",
            "🎛", "⏱", "⏲", "⏰", "🕰", "⌛", "⏳", "📡", "🔋", "🔌", "💡", "🔦", "🕯", "🪔", "🧯",
            "🛢", "💸", "💵", "💴", "💶", "💷", "🪙", "💰", "💳", "💎", "⚖️", "🪜", "🧰", "🔧",
            "🔨", "⚒", "🛠", "⛏", "🪛", "🔩", "⚙️", "🪤", "🧱", "⛓", "🧲", "🔫", "💣", "🧨", "🪓",
            "🔪", "🗡", "⚔️", "🛡", "🚬", "⚰️", "🪦", "⚱️", "🏺", "🔮", "📿", "🧿", "💈", "⚗️", "🔭",
            "🔬", "🕳", "🩹", "🩺", "💊", "💉", "🧬", "🦠", "🧫", "🧪", "🌡", "🧹", "🪣", "🧴", "🧷",
            "🧺", "🧻", "🧼", "🪥", "🪒", "🧽", "🧯", "🛒",
        ],
    ),
    (
        "symbols",
        &[
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🤎", "🖤", "🤍", "💔", "❣️", "💕", "💞", "💓",
            "💗", "💖", "💘", "💝", "💟", "☮️", "✝️", "☪️", "🕉", "☸️", "✡️", "🔯", "🕎", "☯️",
            "☦️", "🛐", "⛎", "♈", "♉", "♊", "♋", "♌", "♍", "♎", "♏", "♐", "♑", "♒",
            "♓", "🆔", "⚛️", "🉑", "☢️", "☣️", "📴", "📳", "🈶", "🈚", "🈸", "🈺", "🈷️", "✴️",
            "🆚", "💮", "🉐", "㊙️", "㊗️", "🈴", "🈵", "🈹", "🈲", "🅰️", "🅱️", "🆎", "🆑", "🅾️",
            "🆘", "❌", "⭕", "🛑", "⛔", "📛", "🚫", "💯", "💢", "♨️", "🚷", "🚯", "🚳", "🚱",
            "🔞", "📵", "🚭", "❗", "❕", "❓", "❔", "‼️", "⁉️", "🔅", "🔆", "〽️", "⚠️", "🚸",
            "🔱", "⚜️", "🔰", "♻️", "✅", "🈯", "💹", "❇️", "✳️", "❎", "🌐", "💠", "Ⓜ️", "🌀",
            "💤", "🏧", "🚾", "♿", "🅿️", "🛗", "🈳", "🈂️", "🛂", "🛃", "🛄", "🛅", "🚹", "🚺",
            "🚼", "⚧", "🚻", "🚮", "🎦", "📶", "🈁", "🔣", "ℹ️", "🔤", "🔡", "🔠", "🆖", "🆗",
            "🆙", "🆒", "🆕", "🆓", "0️⃣", "1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣",
            "🔟",
        ],
    ),
];

/// Filter the picker grid to glyphs matching `query`.
///
/// Empty query: returns all glyphs across categories in their static
/// order with no dedupe. Non-empty query: case-insensitive prefix
/// match against the *category name* (rough categorisation) — the
/// emoji picker today doesn't index per-glyph names because the
/// payload would balloon, but matching on category lets a user type
/// `na` and see the nature shelf. A follow-up can wire a full
/// per-glyph name index without changing the picker's signature.
///
/// `recent` is the per-channel recency from
/// `ClientHandle::recent_reactions(channel)`. It's deduped against
/// the static categories so a glyph never appears twice on screen.
pub fn search<'a>(query: &str, recent: &'a [String]) -> Vec<&'a str> {
    // Borrow recents as &str; static categories already give us &str.
    let recent_refs: Vec<&str> = recent.iter().map(|s| s.as_str()).collect();

    if query.is_empty() {
        let mut out: Vec<&str> = recent_refs.clone();
        for (_, glyphs) in EMOJI_CATEGORIES {
            for g in glyphs.iter().copied() {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        return out;
    }

    let q = query.to_ascii_lowercase();
    let mut out: Vec<&str> = Vec::new();
    // Recents always match if their category contains the query — but
    // since we don't have per-glyph names, surface them only when the
    // query is empty (handled above). For a name query, fall through
    // to the category match.
    for (label, glyphs) in EMOJI_CATEGORIES {
        if label.starts_with(&q) {
            for g in glyphs.iter().copied() {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
    }

    // SAFETY note about lifetimes: recent_refs binds to `recent`'s
    // lifetime via the input slice. Drop it explicitly to silence
    // an unused-variable warning when the empty-query path didn't fire.
    drop(recent_refs);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_lists_all_glyphs_recents_first() {
        let recent = vec!["⭐".to_string(), "🍃".to_string()];
        let glyphs = search("", &recent);
        assert_eq!(glyphs[0], "⭐", "recents must lead the empty-query list");
        assert_eq!(glyphs[1], "🍃");
        // Static categories follow with no dedupe across categories
        // (each glyph appears once); the smileys category follows
        // recents in the static order.
        assert!(
            glyphs.contains(&"😀"),
            "static smileys must appear after recents"
        );
    }

    #[test]
    fn category_prefix_search_filters_grid() {
        let glyphs = search("na", &[]);
        // `na` matches `nature` only — no smileys / food / etc.
        assert!(glyphs.contains(&"🍃"), "nature glyphs must appear");
        assert!(
            !glyphs.contains(&"😀"),
            "smileys must NOT appear under `na` prefix match"
        );
    }

    #[test]
    fn search_is_case_insensitive() {
        let upper = search("FOOD", &[]);
        let lower = search("food", &[]);
        assert_eq!(upper, lower, "case must not affect the match");
        assert!(upper.contains(&"🍕"));
    }

    #[test]
    fn dedupe_against_recents() {
        // A recent that's also in a static category must not appear
        // twice in the empty-query result.
        let recent = vec!["🍃".to_string()];
        let glyphs = search("", &recent);
        let count = glyphs.iter().filter(|g| **g == "🍃").count();
        assert_eq!(count, 1, "recent + static must dedupe to a single entry");
    }
}
