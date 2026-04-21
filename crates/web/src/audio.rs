//! Audio helpers — the willow-chime player.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/notifications.md` §Sound.
//!
//! One global `HtmlAudioElement` loads `/willow-chime.webm` on first
//! use and enforces max queue depth 1 (in-flight + next) — further
//! arrivals replace the queued sample with the newest.

use std::cell::Cell;
use std::rc::Rc;

use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;

/// Path to the bundled chime asset, served by trunk.
pub const CHIME_PATH: &str = "/willow-chime.webm";

/// The chime player. Cloneable handle — construct once and store in
/// Leptos context via [`provide_chime_player`].
#[derive(Clone)]
pub struct ChimePlayer {
    /// The shared audio element.
    audio: SendWrapper<Rc<web_sys::HtmlAudioElement>>,
    /// True when a chime is currently playing.
    playing: SendWrapper<Rc<Cell<bool>>>,
    /// True when a replacement is queued behind the in-flight sample.
    /// Enforces max queue depth 1 (spec: "further arrivals replace the
    /// queued sample with the newest" — there is no stacking).
    queued: SendWrapper<Rc<Cell<bool>>>,
}

impl ChimePlayer {
    /// Construct a chime player. Does not yet load the asset — the
    /// browser will preload when the audio element is first touched.
    pub fn new() -> Self {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("document present");
        let audio: web_sys::HtmlAudioElement = document
            .create_element("audio")
            .expect("audio element")
            .dyn_into()
            .expect("audio type");
        audio.set_src(CHIME_PATH);
        audio.set_preload("auto");
        // Modest default volume — the chime is meant to be soft, not
        // startling. Per spec: "short, low, warm tone … intentionally
        // soft."
        audio.set_volume(0.6);
        Self {
            audio: SendWrapper::new(Rc::new(audio)),
            playing: SendWrapper::new(Rc::new(Cell::new(false))),
            queued: SendWrapper::new(Rc::new(Cell::new(false))),
        }
    }

    /// Play the chime. If a chime is already playing, the request is
    /// queued (at most 1 queued sample); any further requests replace
    /// the queued one. Silently no-ops if the browser cannot load or
    /// decode the asset.
    pub fn play(&self) {
        if self.playing.get() {
            // Already in-flight — mark one queued so the `ended` handler
            // plays it when the current sample finishes.
            self.queued.set(true);
            return;
        }
        self.dispatch_play();
    }

    fn dispatch_play(&self) {
        self.playing.set(true);
        let audio = (*self.audio).clone();
        audio.set_current_time(0.0);
        let promise = audio.play();
        let this = self.clone();
        // Attach an ended handler once — needs to fire for every
        // playback, so re-assign each dispatch.
        let on_ended = wasm_bindgen::closure::Closure::once_into_js(move || {
            this.playing.set(false);
            if this.queued.get() {
                this.queued.set(false);
                this.dispatch_play();
            }
        });
        audio.set_onended(Some(on_ended.unchecked_ref()));
        // Swallow the play() Promise — any rejection (autoplay policy,
        // missing asset) means no sound, no error toast; dev-build
        // console warning is sufficient.
        if let Ok(promise) = promise {
            let this = self.clone();
            let on_err: wasm_bindgen::closure::Closure<dyn FnMut(wasm_bindgen::JsValue)> =
                wasm_bindgen::closure::Closure::new(move |_err: wasm_bindgen::JsValue| {
                    this.playing.set(false);
                    this.queued.set(false);
                });
            let _ = promise.catch(&on_err);
            on_err.forget();
        } else {
            self.playing.set(false);
        }
    }
}

impl Default for ChimePlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide a [`ChimePlayer`] in the current reactive scope.
pub fn provide_chime_player() -> ChimePlayer {
    if let Some(existing) = leptos::prelude::use_context::<ChimePlayer>() {
        return existing;
    }
    let player = ChimePlayer::new();
    leptos::prelude::provide_context(player.clone());
    player
}

/// Retrieve the ambient [`ChimePlayer`]. Returns `None` in contexts
/// (like tests) where one was never provided.
pub fn use_chime_player() -> Option<ChimePlayer> {
    leptos::prelude::use_context::<ChimePlayer>()
}
