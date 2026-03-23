use leptos::prelude::*;

use crate::app::ClientHandle;

/// List of all permission names that can be toggled on a role.
const PERMISSION_NAMES: &[&str] = &[
    "SyncProvider",
    "ManageChannels",
    "ManageRoles",
    "KickMembers",
    "SendMessages",
    "CreateInvite",
    "Administrator",
];

/// A single role entry: (role_id, role_name, set of granted permission strings).
type RoleEntry = (String, String, Vec<String>);

/// Role management panel. Shows all roles with permission toggles,
/// create/delete controls, and role assignment.
///
/// Only the server owner sees management controls (create, delete,
/// permission toggles, assign). Non-owners see a read-only list.
#[component]
pub fn RoleManager(
    client: ClientHandle,
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
) -> impl IntoView {
    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());
    let (assign_peer, set_assign_peer) = signal(String::new());

    // Determine if the local user is the server owner.
    let client_owner = client.clone();
    let is_owner = move || {
        let c = client_owner.borrow();
        let pid = peer_id.get();
        c.state().event_state.owner == pid
    };

    // Create role handler.
    let client_create = client.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        if !name.is_empty() {
            let mut c = client_create.borrow_mut();
            let _ = c.create_role(&name);
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
                    let owner_check = is_owner.clone();
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
                    let role_id_perms = role_id.clone();
                    let role_id_assign = role_id.clone();
                    let client_delete = client.clone();
                    let client_perm = client.clone();
                    let client_assign = client.clone();
                    let owner_check = is_owner.clone();
                    view! {
                        <div class="role-item">
                            <div class="role-item-header">
                                <span class="role-name">{role_name}</span>
                                {
                                    let oc = owner_check.clone();
                                    let cd = client_delete.clone();
                                    let rid = role_id_delete.clone();
                                    move || {
                                        if oc() {
                                            let cd = cd.clone();
                                            let rid = rid.clone();
                                            Some(view! {
                                                <button
                                                    class="role-delete-btn"
                                                    title="Delete role"
                                                    on:click=move |_| {
                                                        let mut c = cd.borrow_mut();
                                                        let _ = c.delete_role(&rid);
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
                                    let oc = owner_check.clone();
                                    let cp = client_perm.clone();
                                    let rid = role_id_perms.clone();
                                    let perms = permissions.clone();
                                    PERMISSION_NAMES.iter().map(|perm_name| {
                                        let perm = perm_name.to_string();
                                        let perm_label = perm.clone();
                                        let perm_check = perm.clone();
                                        let perm_toggle = perm.clone();
                                        let rid_t = rid.clone();
                                        let cp_t = cp.clone();
                                        let oc_t = oc.clone();
                                        let checked = perms.contains(&perm_check);
                                        view! {
                                            <label class="permission-toggle">
                                                <input
                                                    type="checkbox"
                                                    prop:checked=checked
                                                    prop:disabled=move || !oc_t()
                                                    on:change=move |ev| {
                                                        let granted = event_target_checked(&ev);
                                                        let mut c = cp_t.borrow_mut();
                                                        let _ = c.set_permission(&rid_t, &perm_toggle, granted);
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
                                let oc = owner_check.clone();
                                let ca = client_assign.clone();
                                let rid = role_id_assign.clone();
                                move || {
                                    if oc() {
                                        let ca = ca.clone();
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
                                                            let mut c = ca.borrow_mut();
                                                            let _ = c.assign_role(pid.trim(), &rid);
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
