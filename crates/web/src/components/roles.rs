use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::ConfirmDialog;
use crate::state::AppState;

/// List of all permission names that can be toggled on a role.
const PERMISSION_NAMES: &[&str] = &[
    "SyncProvider",
    "ManageChannels",
    "ManageRoles",
    "KickMembers",
    "SendMessages",
    "CreateInvite",
];

/// A single role entry: (role_id, role_name, set of granted permission strings).
type RoleEntry = (String, String, Vec<String>);

/// Role management panel. Shows all roles with permission toggles,
/// create/delete controls, and role assignment.
///
/// Only the server owner sees management controls (create, delete,
/// permission toggles, assign). Non-owners see a read-only list.
#[allow(dead_code)]
#[component]
pub fn RoleManager(
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<AppState>().unwrap();

    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());
    let (assign_peer, set_assign_peer) = signal(String::new());

    // Role delete confirmation state.
    let (show_del_confirm, set_show_del_confirm) = signal(false);
    let (pending_del_role, set_pending_del_role) = signal(Option::<(String, String)>::None);
    let handle_del_confirm = handle.clone();

    // Determine if the local user is the server owner.
    let server_owner_signal = app_state.server.server_owner;
    let is_owner = move || {
        let pid = peer_id.get();
        server_owner_signal.get() == pid
    };

    // Create role handler.
    let handle_create = handle.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        if !name.is_empty() {
            let h = handle_create.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = h.create_role(&name).await;
            });
        }
        set_new_name.set(String::new());
        set_creating.set(false);
    };

    let on_create_keydown = {
        let submit = on_create_submit.clone();
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" {
                ev.prevent_default();
                submit();
            } else if ev.key() == "Escape" {
                set_creating.set(false);
                set_new_name.set(String::new());
            }
        }
    };

    view! {
        <div class="role-manager">
            <div class="role-manager-header">
                <span class="role-manager-title">"ROLES"</span>
                {
                    let owner_check = is_owner;
                    move || {
                        if owner_check() {
                            Some(view! {
                                <button
                                    class="role-add-btn"
                                    title="Create role"
                                    on:click=move |_| set_creating.set(true)
                                >
                                    "+"
                                </button>
                            })
                        } else {
                            None
                        }
                    }
                }
            </div>

            // Inline create input (same pattern as channel create).
            {
                let on_create_keydown = on_create_keydown.clone();
                move || {
                    if creating.get() {
                        let kd = on_create_keydown.clone();
                        Some(view! {
                            <div class="role-create-input">
                                <input
                                    type="text"
                                    placeholder="role name"
                                    prop:value=move || new_name.get()
                                    on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                    on:keydown=kd
                                    on:blur=move |_| {
                                        set_creating.set(false);
                                        set_new_name.set(String::new());
                                    }
                                />
                            </div>
                        })
                    } else {
                        None
                    }
                }
            }

            // Role list.
            <For
                each=move || roles.get()
                key=|(id, _, _)| id.clone()
                let:role
            >
                {
                    let (role_id, role_name, permissions) = role;
                    let role_id_delete = role_id.clone();
                    let role_name_delete = role_name.clone();
                    let role_id_perms = role_id.clone();
                    let role_id_assign = role_id.clone();
                    let handle_perm = handle.clone();
                    let handle_assign = handle.clone();
                    let owner_check = is_owner;
                    view! {
                        <div class="role-item">
                            <div class="role-item-header">
                                <span class="role-name">{role_name}</span>
                                {
                                    let oc = owner_check;
                                    let rid = role_id_delete.clone();
                                    let rname = role_name_delete.clone();
                                    move || {
                                        if oc() {
                                            let rid = rid.clone();
                                            let rname = rname.clone();
                                            Some(view! {
                                                <button
                                                    class="role-delete-btn"
                                                    title="Delete role"
                                                    on:click=move |_| {
                                                        set_pending_del_role.set(Some((rid.clone(), rname.clone())));
                                                        set_show_del_confirm.set(true);
                                                    }
                                                >
                                                    "x"
                                                </button>
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                }
                            </div>

                            // Permission toggles.
                            <div class="permission-toggles">
                                {
                                    let oc = owner_check;
                                    let hp = handle_perm.clone();
                                    let rid = role_id_perms.clone();
                                    let perms = permissions.clone();
                                    PERMISSION_NAMES.iter().map(|perm_name| {
                                        let perm = perm_name.to_string();
                                        let perm_label = perm.clone();
                                        let perm_check = perm.clone();
                                        let perm_toggle = perm.clone();
                                        let rid_t = rid.clone();
                                        let hp_t = hp.clone();
                                        let oc_t = oc;
                                        let checked = perms.contains(&perm_check);
                                        view! {
                                            <label class="permission-toggle">
                                                <input
                                                    type="checkbox"
                                                    prop:checked=checked
                                                    prop:disabled=move || !oc_t()
                                                    on:change=move |ev| {
                                                        let granted = event_target_checked(&ev);
                                                        let h = hp_t.clone();
                                                        let rid = rid_t.clone();
                                                        let perm = perm_toggle.clone();
                                                        wasm_bindgen_futures::spawn_local(async move {
                                                            let _ = h.set_permission(&rid, &perm, granted).await;
                                                        });
                                                    }
                                                />
                                                <span>{perm_label}</span>
                                            </label>
                                        }
                                    }).collect_view()
                                }
                            </div>

                            // Assign role to peer (owner only).
                            {
                                let oc = owner_check;
                                let ha = handle_assign.clone();
                                let rid = role_id_assign.clone();
                                move || {
                                    if oc() {
                                        let ha = ha.clone();
                                        let rid = rid.clone();
                                        Some(view! {
                                            <div class="role-assign">
                                                <input
                                                    type="text"
                                                    class="role-assign-input"
                                                    placeholder="Peer ID to assign"
                                                    prop:value=move || assign_peer.get()
                                                    on:input=move |ev| set_assign_peer.set(event_target_value(&ev))
                                                />
                                                <button
                                                    class="btn btn-sm"
                                                    on:click=move |_| {
                                                        let pid = assign_peer.get_untracked();
                                                        if !pid.trim().is_empty() {
                                                            if let Ok(eid) = pid.trim().parse::<willow_identity::EndpointId>() {
                                                                let h = ha.clone();
                                                                let r = rid.clone();
                                                                wasm_bindgen_futures::spawn_local(async move {
                                                                    let _ = h.assign_role(eid, &r).await;
                                                                });
                                                            }
                                                            set_assign_peer.set(String::new());
                                                        }
                                                    }
                                                >
                                                    "Assign"
                                                </button>
                                            </div>
                                        })
                                    } else {
                                        None
                                    }
                                }
                            }
                        </div>
                    }
                }
            </For>

            // Empty state.
            {move || {
                if roles.get().is_empty() {
                    Some(view! {
                        <div class="empty-state" style="font-size: 12px;">"No roles defined"</div>
                    })
                } else {
                    None
                }
            }}
            <ConfirmDialog
                visible=show_del_confirm
                title="Delete Role"
                message=Signal::derive(move || {
                    pending_del_role.get()
                        .map(|(_, name)| format!("Delete role \"{}\"?", name))
                        .unwrap_or_default()
                })
                confirm_text="Delete"
                danger=true
                on_confirm=Callback::new(move |_| {
                    if let Some((rid, _)) = pending_del_role.get_untracked() {
                        let h = handle_del_confirm.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let _ = h.delete_role(&rid).await;
                        });
                    }
                    set_pending_del_role.set(None);
                    set_show_del_confirm.set(false);
                })
                on_cancel=Callback::new(move |_| {
                    set_pending_del_role.set(None);
                    set_show_del_confirm.set(false);
                })
            />
        </div>
    }
}

/// Extract the checked state from a checkbox change event.
fn event_target_checked(ev: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}
