//! # Day separator
//!
//! Renders a full-width row between messages whose local dates differ.
//! Labels follow `docs/specs/2026-04-19-ui-design/message-row.md`
//! §Day separator:
//!
//! * `— today —`
//! * `— yesterday —`
//! * `— friday · 14 april —` (older than yesterday, within this year)
//! * `— friday · 14 april · 2025 —` (prior year)
//!
//! All text is lowercase. Bucketing happens in the local timezone via
//! `js_sys::Date` (the web UI only ships as WASM in production). The
//! module exports a plain `DayBucket` enum plus the `day_bucket`
//! constructor so `MessageList` can compare consecutive messages with a
//! cheap `PartialEq`.
//!
//! The component itself is deliberately dumb: given a `DayBucket`, it
//! renders the flanked em-dash label. Styling is owned by
//! `style.css` — `.day-separator`, `.day-separator .rule`,
//! `.day-separator em`.

use leptos::prelude::*;

/// Which local-date bucket a timestamp falls into, relative to "now".
///
/// `PartialEq` drives the separator-insertion check in `MessageList`:
/// emit a separator whenever `day_bucket(curr)` differs from
/// `day_bucket(prev)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DayBucket {
    /// Same local date as `now`.
    Today,
    /// Exactly one local-day before `now`.
    Yesterday,
    /// Older than yesterday but within the same calendar year as `now`.
    ThisYear {
        weekday: &'static str,
        day: u32,
        month: &'static str,
    },
    /// Older than the current calendar year.
    Older {
        weekday: &'static str,
        day: u32,
        month: &'static str,
        year: i32,
    },
}

impl DayBucket {
    /// The label text rendered between em-dashes. Always lowercase.
    pub fn label(&self) -> String {
        match self {
            DayBucket::Today => "today".to_string(),
            DayBucket::Yesterday => "yesterday".to_string(),
            DayBucket::ThisYear {
                weekday,
                day,
                month,
            } => format!("{weekday} · {day} {month}"),
            DayBucket::Older {
                weekday,
                day,
                month,
                year,
            } => format!("{weekday} · {day} {month} · {year}"),
        }
    }
}

const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Maximum timestamp value (in milliseconds) representable as a valid
/// JavaScript `Date`. ECMA-262 caps the range at ±100,000,000 days from
/// the epoch; outside this band `Date` accessors return `NaN`, which
/// silently casts to `0` in Rust and would mis-bucket the row. See
/// <https://tc39.es/ecma262/#sec-time-values-and-time-range>.
const MAX_VALID_TS_MS: i64 = 8_640_000_000_000_000;

/// Compute the [`DayBucket`] for a millisecond timestamp in the local
/// timezone. Uses `js_sys::Date` on WASM; on native builds the function
/// returns `DayBucket::Today` so the module still compiles for
/// `cargo check` — only the wasm-pack browser tier exercises the real
/// bucketing logic.
#[cfg(target_arch = "wasm32")]
pub fn day_bucket(ts_ms: u64) -> DayBucket {
    use wasm_bindgen::JsValue;

    // Out-of-range timestamps make `js_sys::Date` accessors return NaN;
    // the `as i32` / `as u32` casts then collapse to 0, which would index
    // into `WEEKDAYS`/`MONTHS` and produce a bogus "sunday · 0 january"
    // label. Bail to a stable fallback variant instead.
    if ts_ms > MAX_VALID_TS_MS as u64 {
        return DayBucket::Older {
            weekday: "unknown",
            day: 0,
            month: "unknown",
            year: 0,
        };
    }

    let ts = js_sys::Date::new(&JsValue::from_f64(ts_ms as f64));
    let now = js_sys::Date::new_0();

    let ts_y = ts.get_full_year() as i32;
    let ts_m = ts.get_month() as u32;
    let ts_d = ts.get_date();

    let now_y = now.get_full_year() as i32;
    let now_m = now.get_month() as u32;
    let now_d = now.get_date();

    if (ts_y, ts_m, ts_d) == (now_y, now_m, now_d) {
        return DayBucket::Today;
    }

    // Build "yesterday" by subtracting one full day from `now` in local
    // time. Using `js_sys::Date::new_with_year_month_day_hr_min_sec_milli`
    // keeps the arithmetic DST-correct because the Date constructor
    // normalises the underlying epoch.
    let yesterday = js_sys::Date::new_with_year_month_day_hr_min_sec_milli(
        now.get_full_year(),
        now.get_month() as i32,
        (now.get_date() as i32) - 1,
        0,
        0,
        0,
        0,
    );
    let y_y = yesterday.get_full_year() as i32;
    let y_m = yesterday.get_month() as u32;
    let y_d = yesterday.get_date();
    if (ts_y, ts_m, ts_d) == (y_y, y_m, y_d) {
        return DayBucket::Yesterday;
    }

    let weekday = WEEKDAYS[ts.get_day() as usize];
    let month = MONTHS[ts_m as usize];

    if ts_y == now_y {
        DayBucket::ThisYear {
            weekday,
            day: ts_d,
            month,
        }
    } else {
        DayBucket::Older {
            weekday,
            day: ts_d,
            month,
            year: ts_y,
        }
    }
}

/// Native fallback so `cargo check` compiles the module outside WASM.
/// Never called in production — `willow-web` only ships as WASM.
#[cfg(not(target_arch = "wasm32"))]
pub fn day_bucket(_ts_ms: u64) -> DayBucket {
    DayBucket::Today
}

/// Full-width row separating messages from different local dates.
///
/// Renders as `— <label> —` flanked by two 1 px rules. The surrounding
/// container (`.day-separator`) uses flex so the rules stretch to fill
/// available width.
#[component]
pub fn DaySeparator(bucket: DayBucket) -> impl IntoView {
    let label = bucket.label();
    let aria = label.clone();
    view! {
        <div class="day-separator" role="separator" aria-label=aria>
            <span class="rule" aria-hidden="true"></span>
            <em>{format!("— {label} —")}</em>
            <span class="rule" aria-hidden="true"></span>
        </div>
    }
}
