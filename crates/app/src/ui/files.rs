//! File sharing UI systems.

use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkCommandSender};

use super::components::*;
use super::resources::*;

/// Open a file dialog when the "Share" button is clicked.
pub fn handle_share_file_button(
    query: Query<&Interaction, (Changed<Interaction>, With<ShareFileButton>)>,
    picker: Res<FilePicker>,
) {
    for interaction in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let rx_arc = picker.rx.clone();

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            if let Ok(mut guard) = rx_arc.lock() {
                *guard = Some(rx);
            }
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    if let Ok(data) = std::fs::read(&path) {
                        let filename = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let mime = mime_from_extension(&filename);
                        let _ = tx.send((filename, mime, data));
                    }
                }
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = rx_arc;
            info!("file picker not yet available on WASM");
        }
    }
}

/// Poll for file picker results and send the ShareFile command.
#[allow(clippy::too_many_arguments)]
pub fn poll_file_picker(
    picker: Res<FilePicker>,
    net_cmd: Res<NetworkCommandSender>,
    server_state: Res<ServerState>,
    mut chat_state: ResMut<ChatState>,
    profiles: Res<ProfileStore>,
    identity: Res<LocalIdentity>,
    db: Res<MessageDbRes>,
) {
    let Ok(mut guard) = picker.rx.lock() else {
        return;
    };
    let Some(rx) = guard.as_ref() else {
        return;
    };

    let Ok((filename, mime_type, data)) = rx.try_recv() else {
        return;
    };

    *guard = None;

    let channel_name = chat_state.current_channel.clone();
    let topic = server_state
        .topic_for_name(&channel_name)
        .unwrap_or(channel_name);

    let size_kb = data.len() / 1024;

    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::ShareFile {
            topic: topic.clone(),
            filename: filename.clone(),
            mime_type,
            data,
        });

    let author = profiles.display_name(&identity.0.peer_id().to_string());
    let body = format!("[shared file: {filename} ({size_kb} KB)]");
    let chat_msg = ChatMessage::new(
        topic,
        author.clone(),
        body.clone(),
        true,
        chat_state.hlc.latest().millis,
    );

    if let Some(ref db_arc) = db.0 {
        if let Ok(db_lock) = db_arc.lock() {
            db_lock.insert(&crate::storage::StoredMessage {
                topic: chat_msg.topic.clone(),
                author,
                body,
                is_local: true,
                timestamp_ms: chat_state.hlc.latest().millis,
                msg_id: String::new(),
            });
        }
    }

    chat_state.messages.push(chat_msg);
    chat_state.messages_dirty = true;
}

/// Guess MIME type from file extension.
#[cfg(not(target_arch = "wasm32"))]
fn mime_from_extension(filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "rs" => "text/x-rust",
        _ => "application/octet-stream",
    }
    .to_string()
}
