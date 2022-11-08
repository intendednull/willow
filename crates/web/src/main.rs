use std::{ops::Deref, rc::Rc};

pub use yew::prelude::*;
pub use yewdux::prelude::*;
pub use yewdux_input::input_value;

#[derive(Default, PartialEq, Clone, Debug)]
pub struct Item {
    message: String,
}

#[derive(Store, Default, PartialEq, Clone, Debug)]
pub struct Timeline {
    history: Vec<Item>,
}

#[derive(PartialEq, Clone, Debug)]
enum Action {
    AddItem(Item),
}

impl Reducer<Timeline> for Action {
    fn apply(self, mut timeline: Rc<Timeline>) -> Rc<Timeline> {
        let state = Rc::make_mut(&mut timeline);

        match self {
            Action::AddItem(item) => state.history.push(item),
        }

        timeline
    }
}

#[function_component]
fn ViewTimeline() -> Html {
    let timeline = use_store_value::<Timeline>();
    let items = timeline
        .history
        .iter()
        .map(|item| {
            html! {
                <p>{&item.message}</p>
            }
        })
        .collect::<Html>();

    html! {
        <div>
            { items }
        </div>
    }
}

#[function_component]
fn InputItem() -> Html {
    let value = use_state(String::default);
    let oninput = {
        let value = value.clone();
        Callback::from(move |e| {
            value.set(input_value(e).unwrap());
        })
    };
    let onclick = {
        let value = value.clone();
        Dispatch::<Timeline>::new().apply_callback(move |_| {
            let message = value.deref().clone();
            value.set(String::default());

            Action::AddItem(Item { message })
        })
    };

    html! {
        <div>
            <textarea {oninput} value={value.deref().clone()} />
            <button {onclick}>{"Submit"}</button>
        </div>
    }
}

#[function_component]
fn App() -> Html {
    html! {
        <div>
            <InputItem />
            <ViewTimeline />
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
