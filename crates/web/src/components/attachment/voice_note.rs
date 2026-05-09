//! `<AttachmentVoiceNote>` — voice-note playback card per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §Voice note.
//!
//! Lays out the spec card (`--bg-2` on `--line`, radius 10 px,
//! `10/12` padding, `max-width: 420px` desktop), with:
//!
//! - Play / pause IconBtn (32×32, `--moss-2` foreground).
//! - Waveform strip (fixed 24 px tall, `--moss-1` bars on `--bg-1`,
//!   with a `--moss-2` progress fill on the played portion).
//! - Timer in the spec mono font (`mm:ss / mm:ss`).
//!
//! Single-instance playback is coordinated via
//! [`crate::voice_note_player::VoiceNotePlayer`]: each card writes
//! its own id when it starts, and watches the shared signal so a
//! competing claim pauses our `<audio>` element.
//!
//! Bytes flow: on mount we fetch the blob, build a `blob:` Object
//! URL on the card's `<audio>`, and (in parallel) decode the same
//! bytes via `AudioContext.decodeAudioData` to extract a tiny peak
//! summary used to draw the waveform. Decode failures fall back to a
//! flat baseline waveform — playback still works because the
//! `<audio>` element handles its own decode.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;
use crate::icons;
use crate::voice_note_player::use_voice_note_player;

/// Number of waveform bars per card. 64 keeps the SVG cheap to
/// render and lines up with the spec's "fixed height 24 px" strip
/// at typical desktop widths.
const WAVEFORM_BARS: usize = 64;

/// Voice-note card. Fetches the blob, decodes it for waveform peaks,
/// and drives playback through a card-local `<audio>` element while
/// the [`crate::voice_note_player::VoiceNotePlayer`] context arbiters
/// single-instance.
///
/// Stable id derives from the blob hash + filename so each row in the
/// chat keeps the same identity across re-renders. The hash alone is
/// content-addressed (great for dedup) but two messages quoting the
/// same file would share an id and pause each other on play; pairing
/// with the filename keeps that edge-case sane.
#[component]
pub fn AttachmentVoiceNote(
    /// Hex-encoded blob hash from `EventKind::FileMessage::hash`.
    hash: String,
    /// Sender-declared filename, surfaced in the IconBtn's
    /// `aria-label` so screen readers identify which clip is being
    /// played.
    filename: String,
    /// Sender-declared size in bytes — display only. Reserved for a
    /// future "Xs · 12 KB" subtitle row; today we don't render it.
    size_bytes: u64,
) -> impl IntoView {
    let _ = size_bytes;
    let id = format!("vn-{}-{}", &hash[..hash.len().min(12)], filename);
    let player = use_voice_note_player();
    let handle = use_context::<WebClientHandle>();
    let audio_ref = NodeRef::<leptos::html::Audio>::new();

    let (src, set_src) = signal(String::new());
    let (peaks, set_peaks) = signal::<Vec<f32>>(vec![0.05; WAVEFORM_BARS]);
    let (current, set_current) = signal(0.0_f64);
    let (duration, set_duration) = signal(0.0_f64);
    let (playing, set_playing) = signal(false);

    // Fetch + decode the blob on mount. Object URL feeds <audio>;
    // decoded peaks feed the waveform. Independent paths — a decode
    // failure leaves the flat baseline waveform but playback still
    // works because <audio> handles its own decode.
    let fetch_hash = hash.clone();
    Effect::new(move |_| {
        let Some(handle) = handle.clone() else {
            return;
        };
        let Some(blob_hash) = willow_client::hex_to_blob_hash(&fetch_hash) else {
            tracing::warn!(hash = %fetch_hash, "AttachmentVoiceNote: malformed hex hash");
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let bytes = match handle.fetch_blob(blob_hash).await {
                Ok(Some(b)) => b,
                Ok(None) => {
                    tracing::warn!("AttachmentVoiceNote: blob not available locally");
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "AttachmentVoiceNote: blob fetch failed");
                    return;
                }
            };
            // Build the playable Object URL first so the user can
            // press play even if waveform decoding takes longer.
            if let Some(url) = bytes_to_object_url(&bytes) {
                let url_for_cleanup = url.clone();
                on_cleanup(move || {
                    let _ = web_sys::Url::revoke_object_url(&url_for_cleanup);
                });
                set_src.set(url);
            }
            decode_peaks_async(bytes, WAVEFORM_BARS, set_peaks);
        });
    });

    // Pause us when somebody else claims the player slot.
    let id_for_effect = id.clone();
    Effect::new(move |_| {
        let active = player.active.get();
        if active.as_deref() != Some(&id_for_effect) {
            if let Some(el) = audio_ref.get() {
                if !el.paused() {
                    let _ = el.pause();
                }
            }
            // Mirror in our own `playing` signal so the icon flips
            // back to `play` even when the pause came from outside.
            set_playing.set(false);
        }
    });

    let id_play = id.clone();
    let on_toggle = move |_ev: web_sys::MouseEvent| {
        let Some(el) = audio_ref.get() else {
            return;
        };
        if el.paused() {
            player.claim(id_play.clone());
            // play() returns a Promise; we don't await — the `play`
            // event will flip `playing` once the engine starts.
            let _ = el.play();
        } else {
            let _ = el.pause();
        }
    };

    let id_pause = id.clone();
    let on_pause = move |_ev: web_sys::Event| {
        set_playing.set(false);
        player.release_if_active(&id_pause);
    };
    let on_play = move |_ev: web_sys::Event| {
        set_playing.set(true);
    };
    let on_loaded_metadata = move |_ev: web_sys::Event| {
        if let Some(el) = audio_ref.get() {
            let dur = el.duration();
            if dur.is_finite() {
                set_duration.set(dur);
            }
        }
    };
    let on_time_update = move |_ev: web_sys::Event| {
        if let Some(el) = audio_ref.get() {
            set_current.set(el.current_time());
        }
    };
    let id_ended = id.clone();
    let on_ended = move |_ev: web_sys::Event| {
        set_playing.set(false);
        set_current.set(0.0);
        player.release_if_active(&id_ended);
    };

    let aria_label_play = format!("play voice note {filename}");
    let aria_label_pause = format!("pause voice note {filename}");
    let aria_label_button = move || {
        if playing.get() {
            aria_label_pause.clone()
        } else {
            aria_label_play.clone()
        }
    };

    let progress_ratio = Memo::new(move |_| {
        let d = duration.get();
        if d <= 0.0 {
            0.0
        } else {
            (current.get() / d).clamp(0.0, 1.0)
        }
    });

    view! {
        <div class="attachment attachment--voice-note">
            <button
                class="attachment__voice-toggle"
                type="button"
                aria-label=aria_label_button
                on:click=on_toggle
            >
                {move || if playing.get() {
                    icons::icon_pause().into_any()
                } else {
                    icons::icon_play().into_any()
                }}
            </button>
            <div class="attachment__voice-body">
                <Waveform peaks=peaks progress=progress_ratio.into() />
                <div class="attachment__voice-timer">
                    {move || format_timer(current.get(), duration.get())}
                </div>
            </div>
            <audio
                node_ref=audio_ref
                class="attachment__voice-audio"
                preload="metadata"
                src=move || src.get()
                on:play=on_play
                on:pause=on_pause
                on:ended=on_ended
                on:loadedmetadata=on_loaded_metadata
                on:timeupdate=on_time_update
            />
        </div>
    }
}

/// The waveform strip — `WAVEFORM_BARS` rect bars in an SVG, sized
/// to fill the parent. Past the playback head, bars are `--moss-1`;
/// before, `--moss-2`. The peaks signal updates once decoding lands;
/// before then we render the flat baseline so the strip never has
/// the "missing chrome" look of an empty SVG.
#[component]
fn Waveform(peaks: ReadSignal<Vec<f32>>, progress: Signal<f64>) -> impl IntoView {
    let bars = move || {
        let p = peaks.get();
        let prog = progress.get();
        let n = p.len().max(1) as f64;
        let bar_w = 100.0 / n;
        let played_bars = (prog * n).round() as usize;
        p.into_iter()
            .enumerate()
            .map(|(i, peak)| {
                let h = (peak.clamp(0.05, 1.0) as f64) * 100.0;
                let y = 50.0 - h / 2.0;
                let x = i as f64 * bar_w;
                let class_attr = if i < played_bars {
                    "attachment__voice-bar attachment__voice-bar--played"
                } else {
                    "attachment__voice-bar"
                };
                view! {
                    <rect
                        class=class_attr
                        x=format!("{x:.3}%")
                        y=format!("{y:.3}%")
                        width=format!("{:.3}%", bar_w * 0.7)
                        height=format!("{h:.3}%")
                        rx="0.5"
                    />
                }
            })
            .collect_view()
    };
    view! {
        <svg
            class="attachment__voice-waveform"
            viewBox="0 0 100 100"
            preserveAspectRatio="none"
            aria-hidden="true"
        >
            {bars}
        </svg>
    }
}

/// `mm:ss / mm:ss` timer per spec. NaN/infinite duration (e.g. while
/// the file is still streaming the metadata header) renders as
/// `--:--` for the total.
pub(crate) fn format_timer(current: f64, duration: f64) -> String {
    let cur = format_mm_ss(current);
    let total = if duration.is_finite() && duration > 0.0 {
        format_mm_ss(duration)
    } else {
        "--:--".to_string()
    };
    format!("{cur} / {total}")
}

fn format_mm_ss(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "0:00".to_string();
    }
    let total = seconds.round() as i64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}

/// Build a `blob:` Object URL for arbitrary audio bytes. We don't
/// pass a MIME type because the `<audio>` element sniffs from the
/// container; that keeps the helper agnostic to whatever codec the
/// sender chose (webm/opus, mp3, ogg, etc.).
fn bytes_to_object_url(data: &[u8]) -> Option<String> {
    let array = js_sys::Uint8Array::from(data);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Spawn the AudioContext decode + peak extraction on the JS event
/// loop. On success, `set_peaks` receives `bars` peaks in `[0.05, 1.0]`.
/// On any failure (decode error, AudioContext unavailable in this
/// browser, etc.) the signal stays at its baseline default.
fn decode_peaks_async(bytes: Vec<u8>, bars: usize, set_peaks: WriteSignal<Vec<f32>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let ctx = match web_sys::AudioContext::new() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(?e, "AttachmentVoiceNote: AudioContext::new failed");
                return;
            }
        };
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let buffer: js_sys::ArrayBuffer = array.buffer().unchecked_into();
        let promise = match ctx.decode_audio_data(&buffer) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(?e, "AttachmentVoiceNote: decode_audio_data failed");
                return;
            }
        };
        let result = wasm_bindgen_futures::JsFuture::from(promise).await;
        // Drop the AudioContext as soon as decoding settles, regardless
        // of outcome — we never play audio through it, so holding it
        // would just leak.
        drop(ctx);
        let buf = match result {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(?e, "AttachmentVoiceNote: decode promise rejected");
                return;
            }
        };
        let Ok(audio_buf) = buf.dyn_into::<web_sys::AudioBuffer>() else {
            return;
        };
        let Ok(channel) = audio_buf.get_channel_data(0) else {
            return;
        };
        let peaks = bucket_peaks(&channel, bars);
        set_peaks.set(peaks);
    });
}

/// Bucket a PCM channel into `bars` peaks normalised to `[0.05, 1.0]`.
/// The 0.05 floor keeps quiet sections visually present so the
/// waveform doesn't disappear into the strip background.
fn bucket_peaks(samples: &[f32], bars: usize) -> Vec<f32> {
    if samples.is_empty() || bars == 0 {
        return vec![0.05; bars];
    }
    let bucket = (samples.len() / bars).max(1);
    let mut out = Vec::with_capacity(bars);
    for b in 0..bars {
        let start = b * bucket;
        let end = ((b + 1) * bucket).min(samples.len());
        let slice = &samples[start..end];
        let mut peak = 0.0_f32;
        for &s in slice {
            let a = s.abs();
            if a > peak {
                peak = a;
            }
        }
        out.push(peak.clamp(0.05, 1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mm_ss_pads_seconds() {
        assert_eq!(format_mm_ss(0.0), "0:00");
        assert_eq!(format_mm_ss(7.0), "0:07");
        assert_eq!(format_mm_ss(60.0), "1:00");
        assert_eq!(format_mm_ss(125.0), "2:05");
    }

    #[test]
    fn format_mm_ss_handles_garbage() {
        assert_eq!(format_mm_ss(f64::NAN), "0:00");
        assert_eq!(format_mm_ss(-3.0), "0:00");
    }

    #[test]
    fn format_timer_uses_dashes_when_duration_unknown() {
        assert_eq!(format_timer(2.0, f64::NAN), "0:02 / --:--");
        assert_eq!(format_timer(0.0, 0.0), "0:00 / --:--");
    }

    #[test]
    fn format_timer_renders_both_sides() {
        assert_eq!(format_timer(15.0, 60.0), "0:15 / 1:00");
    }

    #[test]
    fn bucket_peaks_normalises_to_floor() {
        let samples = vec![0.0; 100];
        let peaks = bucket_peaks(&samples, 10);
        assert_eq!(peaks.len(), 10);
        for p in peaks {
            assert!(
                (p - 0.05).abs() < 1e-6,
                "silence buckets to the visual floor"
            );
        }
    }

    #[test]
    fn bucket_peaks_picks_max_amplitude_per_bucket() {
        let samples = vec![0.1, -0.9, 0.3, 0.4, -0.2, 0.5];
        let peaks = bucket_peaks(&samples, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.9).abs() < 1e-6, "first half max is |-0.9|");
        assert!((peaks[1] - 0.5).abs() < 1e-6, "second half max is 0.5");
    }
}
