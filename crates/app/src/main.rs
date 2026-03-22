use bevy::prelude::*;
use willow_app::network_bridge::NetworkPlugin;
use willow_app::theme;
use willow_app::ui::UiPlugin;

fn main() {
    App::new()
        .insert_resource(ClearColor(theme::MAIN_BG))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Willow".to_string(),
                resolution: (1280u32, 720u32).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(NetworkPlugin)
        .add_plugins(UiPlugin)
        .run();
}
