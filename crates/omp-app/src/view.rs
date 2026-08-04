#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use dioxus::prelude::*;
use futures_util::StreamExt;
use omp_control_client::{ClientConfig, ClientEvent, ConnectionStatus, decode_pairing_link};
use omp_control_protocol::{AgentId, AgentLifecycle};
use omp_rpc::{
    ExtensionUiRequest, ExtensionUiRequestFrame, ExtensionUiResponse, ExtensionUiResponseFrame,
};

use crate::{
    AppActions, AppModel, InteractionLeaseState, PendingInteraction, SavedServerProfile,
    TranscriptKind, client_descriptor, decode_pairing_qr, initial_pairing_link, load_profiles,
    save_profile, start_client,
};

const STYLE: &str = include_str!("style.css");

pub fn app() -> Element {
    let model = use_signal(AppModel::default);
    let actions = use_signal(|| None::<AppActions>);
    let profiles = use_signal(|| load_profiles().unwrap_or_default());
    use_context_provider(|| model);
    use_context_provider(|| actions);
    use_context_provider(|| profiles);

    let connected = actions.read().is_some();
    rsx! {
        style { {STYLE} }
        if connected {
            Workspace {}
        } else {
            ConnectionScreen {}
        }
    }
}

#[component]
fn ConnectionScreen() -> Element {
    let mut model = use_context::<Signal<AppModel>>();
    let actions = use_context::<Signal<Option<AppActions>>>();
    let mut profiles = use_context::<Signal<Vec<SavedServerProfile>>>();
    let mut pairing_link = use_signal(|| initial_pairing_link().unwrap_or_default());
    let mut device_name = use_signal(default_device_name);
    let mut scanning = use_signal(|| false);

    let saved_profiles = profiles.read().clone();
    let error = model.read().error.clone();
    rsx! {
        main { class: "connection-shell",
            section { class: "connection-hero",
                div { class: "brand-mark", "OMP" }
                p { class: "eyebrow", "OH MY PI / REMOTE CONTROL" }
                h1 { "Your agents, wherever you are." }
                p { class: "hero-copy",
                    "Pair once, then follow runs, steer work, and answer agent prompts over one encrypted connection."
                }
                div { class: "security-note",
                    span { class: "pulse" }
                    "Pairing secrets stay inside the URL fragment and encrypted WebSocket."
                }
            }
            section { class: "connection-panel",
                div { class: "panel-heading",
                    p { class: "eyebrow", "CONNECT" }
                    h2 { "Pair this device" }
                    p { "Scan the QR from `ompd pair`, open its deep link, or paste the link below." }
                }

                if let Some(error) = error {
                    div { class: "alert", role: "alert", "{error}" }
                }

                label { class: "field-label", r#for: "device-name", "Device name" }
                input {
                    id: "device-name",
                    class: "text-input",
                    value: "{device_name}",
                    oninput: move |event| device_name.set(event.value()),
                }
                label { class: "field-label", r#for: "pairing-link", "Pairing link or QR payload" }
                textarea {
                    id: "pairing-link",
                    class: "text-area compact",
                    rows: 4,
                    placeholder: "omp-remote://pair#…",
                    value: "{pairing_link}",
                    oninput: move |event| pairing_link.set(event.value()),
                }
                div { class: "pair-actions",
                    label { class: "button secondary file-button",
                        if scanning() { "Reading QR…" } else { "Scan QR image" }
                        input {
                            r#type: "file",
                            accept: "image/png,image/jpeg",
                            capture: "environment",
                            onchange: move |event| async move {
                                scanning.set(true);
                                let mut found = None;
                                for file in event.files() {
                                    match file.read_bytes().await {
                                        Ok(bytes) => {
                                            found = Some(decode_pairing_qr(&bytes));
                                            break;
                                        }
                                        Err(error) => {
                                            model.write().error = Some(format!("Cannot read QR image: {error}"));
                                        }
                                    }
                                }
                                if let Some(result) = found {
                                    match result {
                                        Ok(link) => pairing_link.set(link),
                                        Err(error) => model.write().error = Some(error.to_string()),
                                    }
                                }
                                scanning.set(false);
                            },
                        }
                    }
                    button {
                        class: "button primary",
                        disabled: pairing_link.read().trim().is_empty(),
                        onclick: move |_| {
                            model.write().error = None;
                            let bundle = match decode_pairing_link(&pairing_link.read()) {
                                Ok(bundle) => bundle,
                                Err(error) => {
                                    model.write().error = Some(error.to_string());
                                    return;
                                }
                            };
                            if bundle.expires_at_ms <= unix_time_ms() {
                                model.write().error = Some("This pairing link has expired. Create a new one with `ompd pair`.".to_owned());
                                return;
                            }
                            let profile = SavedServerProfile::from_pairing(&bundle, "OMP server");
                            let config = match ClientConfig::pairing(
                                bundle,
                                client_descriptor(),
                                device_name.read().trim(),
                            ) {
                                Ok(config) => config,
                                Err(error) => {
                                    model.write().error = Some(error.to_string());
                                    return;
                                }
                            };
                            match save_profile(profile) {
                                Ok(saved) => profiles.set(saved),
                                Err(error) => model.write().error = Some(error.to_string()),
                            }
                            connect(config, model, actions);
                        },
                        "Pair and connect"
                    }
                }

                if !saved_profiles.is_empty() {
                    div { class: "saved-servers",
                        div { class: "divider", span { "OR" } }
                        p { class: "field-label", "Saved servers" }
                        for profile in saved_profiles {
                            button {
                                key: "{profile.server_id}",
                                class: "server-row",
                                onclick: move |_| {
                                    model.write().error = None;
                                    connect(profile.client_config(client_descriptor()), model, actions);
                                },
                                span {
                                    strong { "{profile.name}" }
                                    small { "{profile.endpoint}" }
                                }
                                span { class: "arrow", "→" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn connect(
    config: ClientConfig,
    mut model: Signal<AppModel>,
    mut actions_signal: Signal<Option<AppActions>>,
) {
    match start_client(config) {
        Ok((actions, mut events)) => {
            model.write().connection = ConnectionStatus::Connecting;
            actions_signal.set(Some(actions.clone()));
            dioxus::dioxus_core::spawn_forever(async move {
                while let Some(event) = events.next().await {
                    let state = actions.handle().state();
                    model.write().apply_client_event(event.clone(), &state);
                    match event {
                        ClientEvent::ConnectionChanged(ConnectionStatus::Connected { .. }) => {
                            match actions.list_agents().await {
                                Ok(agents) => {
                                    for agent in &agents {
                                        let _ = actions.subscribe(agent.agent_id.clone());
                                    }
                                    model.write().replace_agents(agents);
                                }
                                Err(error) => model.write().error = Some(error.to_string()),
                            }
                        }
                        ClientEvent::InteractionRequest(interaction)
                            if interaction_requires_response(&interaction.request) =>
                        {
                            match actions
                                .acquire_interaction_lease(interaction.agent_id.clone())
                                .await
                            {
                                Ok(_) => model.write().set_interaction_lease(
                                    &interaction.agent_id,
                                    InteractionLeaseState::Owned,
                                ),
                                Err(error) => model.write().set_interaction_lease(
                                    &interaction.agent_id,
                                    InteractionLeaseState::Unavailable(error.to_string()),
                                ),
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        Err(error) => model.write().error = Some(error.to_string()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Agents,
    Devices,
}

#[component]
fn Workspace() -> Element {
    let mut model = use_context::<Signal<AppModel>>();
    let mut actions_signal = use_context::<Signal<Option<AppActions>>>();
    let mut page = use_signal(|| Page::Agents);
    let agents = model.read().agents.values().cloned().collect::<Vec<_>>();
    let selected = model.read().selected_agent.clone();
    let connection = model.read().connection.clone();
    let interaction = model.read().interactions.values().next().cloned();

    rsx! {
        main { class: "app-shell",
            aside { class: "sidebar",
                div { class: "sidebar-brand",
                    div { class: "brand-mark small", "OMP" }
                    div { strong { "Remote" } small { "Control plane" } }
                }
                nav { class: "main-nav", aria_label: "Primary",
                    button {
                        class: if page() == Page::Agents { "nav-item active" } else { "nav-item" },
                        onclick: move |_| page.set(Page::Agents),
                        span { "Agents" }
                        span { class: "nav-count", "{agents.len()}" }
                    }
                    button {
                        class: if page() == Page::Devices { "nav-item active" } else { "nav-item" },
                        onclick: move |_| page.set(Page::Devices),
                        "Devices"
                    }
                }
                if page() == Page::Agents {
                    div { class: "agent-nav",
                        p { class: "nav-section-label", "WORKSPACES" }
                        for agent in agents.iter() {
                            {
                                let agent_id = agent.agent_id.clone();
                                let is_selected = selected.as_ref() == Some(&agent_id);
                                rsx! {
                                    button {
                                        key: "{agent_id}",
                                        class: if is_selected { "agent-nav-item selected" } else { "agent-nav-item" },
                                        onclick: move |_| model.write().selected_agent = Some(agent_id.clone()),
                                        span { class: lifecycle_dot(&agent.lifecycle) }
                                        span { class: "agent-nav-copy",
                                            strong { "{agent.agent_id}" }
                                            small { "{lifecycle_label(&agent.lifecycle)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    LaunchAgentForm {}
                }
                div { class: "sidebar-footer",
                    span { class: connection_class(&connection), "{connection_label(&connection)}" }
                    button {
                        onclick: move |_| {
                            if let Some(actions) = actions_signal.read().clone() {
                                dioxus::dioxus_core::spawn_forever(async move {
                                    let _ = actions.shutdown().await;
                                    actions_signal.set(None);
                                    *model.write() = AppModel::default();
                                });
                            } else {
                                actions_signal.set(None);
                                *model.write() = AppModel::default();
                            }
                        },
                        "Disconnect"
                    }
                }
            }
            section { class: "content",
                if let Some(error) = model.read().error.clone() {
                    div { class: "top-alert", role: "alert",
                        span { "{error}" }
                        button { onclick: move |_| model.write().error = None, "Dismiss" }
                    }
                }
                match page() {
                    Page::Agents => rsx! { AgentPage {} },
                    Page::Devices => rsx! { DevicesPage {} },
                }
            }
            if let Some(interaction) = interaction {
                {
                    let key = interaction_key(&interaction);
                    rsx! { InteractionDialog { key: "{key}", interaction } }
                }
            }
        }
    }
}

#[component]
fn LaunchAgentForm() -> Element {
    let mut model = use_context::<Signal<AppModel>>();
    let actions_signal = use_context::<Signal<Option<AppActions>>>();
    let mut agent_id = use_signal(String::new);
    rsx! {
        form {
            class: "launch-form",
            onsubmit: move |event| {
                event.prevent_default();
                let value = agent_id.read().trim().to_owned();
                let Ok(id) = AgentId::new(value) else {
                    model.write().error = Some("Agent ID cannot be empty.".to_owned());
                    return;
                };
                let Some(actions) = actions_signal.read().clone() else { return };
                model.write().selected_agent = Some(id.clone());
                agent_id.set(String::new());
                spawn(async move {
                    if let Err(error) = actions.launch(id).await {
                        model.write().error = Some(error.to_string());
                    }
                });
            },
            input {
                class: "sidebar-input",
                placeholder: "New agent ID",
                value: "{agent_id}",
                oninput: move |event| agent_id.set(event.value()),
            }
            button { class: "icon-button", r#type: "submit", aria_label: "Launch agent", "+" }
        }
    }
}

#[component]
fn AgentPage() -> Element {
    let model = use_context::<Signal<AppModel>>();
    let actions_signal = use_context::<Signal<Option<AppActions>>>();
    let mut message = use_signal(String::new);
    let mut session_path = use_signal(String::new);
    let selected = model.read().selected_agent.clone();
    let Some(agent_id) = selected else {
        return rsx! {
            div { class: "empty-state",
                p { class: "eyebrow", "NO AGENT SELECTED" }
                h2 { "Launch or select an agent" }
                p { "Agents appear here with authoritative lifecycle and streaming run state." }
            }
        };
    };
    let agent = model.read().agents.get(&agent_id).cloned();
    let transcript = model
        .read()
        .transcripts
        .get(&agent_id)
        .cloned()
        .unwrap_or_default();
    let lifecycle = agent.as_ref().map_or_else(
        || "Launching".to_owned(),
        |agent| lifecycle_label(&agent.lifecycle),
    );

    rsx! {
        header { class: "agent-header",
            div {
                p { class: "eyebrow", "AGENT" }
                h1 { "{agent_id}" }
                div { class: "status-line",
                    span { class: agent.as_ref().map_or("dot idle", |agent| lifecycle_dot(&agent.lifecycle)) }
                    "{lifecycle}"
                    if let Some(agent) = agent.as_ref() {
                        span { class: "revision", "rev {agent.revision.0} · seq {agent.event_sequence.0}" }
                    }
                }
            }
            div { class: "header-actions",
                button {
                    class: "button secondary danger",
                    onclick: {
                        let agent_id = agent_id.clone();
                        move |_| {
                            let id = agent_id.clone();
                            run_unit_action(model, actions_signal, move |actions| async move {
                                actions.stop(id).await
                            });
                        }
                    },
                    "Stop"
                }
                button {
                    class: "button secondary",
                    onclick: {
                        let agent_id = agent_id.clone();
                        move |_| {
                            let id = agent_id.clone();
                            run_unit_action(model, actions_signal, move |actions| async move {
                                actions.abort(id).await
                            });
                        }
                    },
                    "Abort turn"
                }
            }
        }
        section { class: "run-view", aria_live: "polite",
            if transcript.entries.is_empty() && transcript.partial_assistant.is_empty() {
                div { class: "run-placeholder",
                    span { class: "placeholder-glyph", "⌁" }
                    h3 { "Ready for a prompt" }
                    p { "Streaming output, tool activity, and thinking summaries will appear here." }
                }
            }
            for entry in transcript.entries {
                {
                    let key = format!("{}-{:?}", entry.sequence.0, entry.kind);
                    rsx! {
                        article {
                            key: "{key}",
                            class: transcript_class(entry.kind),
                            p { class: "message-label", "{transcript_label(entry.kind)}" }
                            pre { "{entry.text}" }
                        }
                    }
                }
            }
            if !transcript.partial_thinking.is_empty() {
                article { class: "message thinking streaming",
                    p { class: "message-label", "THINKING · LIVE" }
                    pre { "{transcript.partial_thinking}" }
                }
            }
            if !transcript.partial_assistant.is_empty() {
                article { class: "message assistant streaming",
                    p { class: "message-label", "ASSISTANT · LIVE" }
                    pre { "{transcript.partial_assistant}" }
                }
            }
        }
        section { class: "composer",
            textarea {
                class: "composer-input",
                rows: 4,
                placeholder: "Tell the agent what to do next…",
                value: "{message}",
                oninput: move |event| message.set(event.value()),
            }
            div { class: "composer-actions",
                div { class: "secondary-actions",
                    button {
                        class: "button ghost",
                        disabled: message.read().trim().is_empty(),
                        onclick: {
                            let agent_id = agent_id.clone();
                            move |_| submit_message(model, actions_signal, message, agent_id.clone(), MessageAction::Steer)
                        },
                        "Steer now"
                    }
                    button {
                        class: "button ghost",
                        disabled: message.read().trim().is_empty(),
                        onclick: {
                            let agent_id = agent_id.clone();
                            move |_| submit_message(model, actions_signal, message, agent_id.clone(), MessageAction::FollowUp)
                        },
                        "Queue follow-up"
                    }
                }
                button {
                    class: "button primary",
                    disabled: message.read().trim().is_empty(),
                    onclick: {
                        let agent_id = agent_id.clone();
                        move |_| submit_message(model, actions_signal, message, agent_id.clone(), MessageAction::Prompt)
                    },
                    "Send prompt"
                }
            }
            details { class: "session-switcher",
                summary { "Resume another session" }
                div {
                    input {
                        class: "text-input",
                        placeholder: "/path/to/session.jsonl",
                        value: "{session_path}",
                        oninput: move |event| session_path.set(event.value()),
                    }
                    button {
                        class: "button secondary",
                        disabled: session_path.read().trim().is_empty(),
                        onclick: {
                            let agent_id = agent_id.clone();
                            move |_| {
                                let path = session_path.read().trim().to_owned();
                                let id = agent_id.clone();
                                run_unit_action(model, actions_signal, move |actions| async move {
                                    actions.switch_session(id, path).await
                                });
                            }
                        },
                        "Resume"
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum MessageAction {
    Prompt,
    Steer,
    FollowUp,
}

fn submit_message(
    mut model: Signal<AppModel>,
    actions_signal: Signal<Option<AppActions>>,
    mut input: Signal<String>,
    agent_id: AgentId,
    action: MessageAction,
) {
    let value = input.read().trim().to_owned();
    if value.is_empty() {
        return;
    }
    let Some(actions) = actions_signal.read().clone() else {
        return;
    };
    input.set(String::new());
    spawn(async move {
        let result = match action {
            MessageAction::Prompt => actions.prompt(agent_id, value).await.map(|_| ()),
            MessageAction::Steer => actions.steer(agent_id, value).await,
            MessageAction::FollowUp => actions.follow_up(agent_id, value).await,
        };
        if let Err(error) = result {
            model.write().error = Some(error.to_string());
        }
    });
}

fn run_unit_action<F, Fut>(
    mut model: Signal<AppModel>,
    actions_signal: Signal<Option<AppActions>>,
    action: F,
) where
    F: FnOnce(AppActions) -> Fut + 'static,
    Fut: Future<Output = Result<(), crate::ActionError>> + 'static,
{
    let Some(actions) = actions_signal.read().clone() else {
        return;
    };
    spawn(async move {
        if let Err(error) = action(actions).await {
            model.write().error = Some(error.to_string());
        }
    });
}

#[component]
fn DevicesPage() -> Element {
    let mut model = use_context::<Signal<AppModel>>();
    let actions_signal = use_context::<Signal<Option<AppActions>>>();
    let devices = model.read().devices.clone();
    let own_device = model.read().own_device_id.clone();

    use_effect(move || {
        let Some(actions) = actions_signal.read().clone() else {
            return;
        };
        spawn(async move {
            match actions.list_devices().await {
                Ok(devices) => model.write().replace_devices(devices),
                Err(error) => model.write().error = Some(error.to_string()),
            }
        });
    });

    rsx! {
        header { class: "page-header",
            p { class: "eyebrow", "SECURITY" }
            h1 { "Paired devices" }
            p { "Review scopes and revoke credentials that should no longer reach this daemon." }
        }
        section { class: "device-grid",
            if devices.is_empty() {
                div { class: "empty-card", "No device records are available for this credential." }
            }
            for device in devices {
                {
                    let is_self = own_device.as_ref() == Some(&device.device_id);
                    let revoked = device.revoked_at_ms.is_some();
                    let device_id = device.device_id.clone();
                    rsx! {
                        article { key: "{device_id}", class: if revoked { "device-card revoked" } else { "device-card" },
                            div { class: "device-card-head",
                                div {
                                    h3 { "{device.name}" }
                                    p { "{device.platform:?} · {device.device_id}" }
                                }
                                if is_self { span { class: "pill", "This device" } }
                                if revoked { span { class: "pill muted", "Revoked" } }
                            }
                            div { class: "scope-list",
                                {scope_badge("Observe", device.scopes.observe)}
                                {scope_badge("Prompt", device.scopes.prompt)}
                                {scope_badge("Sessions", device.scopes.mutate_session)}
                                {scope_badge("Stop", device.scopes.stop_agent)}
                                {scope_badge("UI", device.scopes.answer_ui)}
                                {scope_badge("Admin", device.scopes.administer_devices)}
                            }
                            div { class: "device-meta",
                                span { "Created {device.created_at_ms}" }
                                span { "Last seen {format_timestamp(device.last_seen_at_ms)}" }
                            }
                            button {
                                class: "button secondary danger",
                                disabled: is_self || revoked,
                                onclick: move |_| {
                                    let Some(actions) = actions_signal.read().clone() else { return };
                                    let id = device_id.clone();
                                    spawn(async move {
                                        match actions.revoke_device(id).await {
                                            Ok(_) => match actions.list_devices().await {
                                                Ok(devices) => model.write().replace_devices(devices),
                                                Err(error) => model.write().error = Some(error.to_string()),
                                            },
                                            Err(error) => model.write().error = Some(error.to_string()),
                                        }
                                    });
                                },
                                if is_self { "Current credential" } else if revoked { "Revoked" } else { "Revoke access" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn InteractionDialog(interaction: PendingInteraction) -> Element {
    let mut model = use_context::<Signal<AppModel>>();
    let actions_signal = use_context::<Signal<Option<AppActions>>>();
    let agent_id = interaction.envelope.agent_id.clone();
    let ExtensionUiRequestFrame::Request { id, request } = interaction.envelope.request.clone();
    let initial = match &request {
        ExtensionUiRequest::Editor { prefill, .. } => prefill.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let mut value = use_signal(|| initial);
    let owned = interaction.lease == InteractionLeaseState::Owned;
    let requires_response = matches!(
        &request,
        ExtensionUiRequest::Select { .. }
            | ExtensionUiRequest::Confirm { .. }
            | ExtensionUiRequest::Input { .. }
            | ExtensionUiRequest::Editor { .. }
    );

    let content = match request {
        ExtensionUiRequest::Select { title, options, .. } => rsx! {
            p { class: "eyebrow", "AGENT QUESTION" }
            h2 { "{title}" }
            div { class: "option-list",
                for option in options {
                    button {
                        class: "option-button",
                        disabled: !owned,
                        onclick: {
                            let id = id.clone();
                            let agent_id = agent_id.clone();
                            move |_| respond_interaction(
                                model,
                                actions_signal,
                                agent_id.clone(),
                                id.clone(),
                                ExtensionUiResponse::Value { value: option.clone() },
                            )
                        },
                        "{option}"
                    }
                }
            }
        },
        ExtensionUiRequest::Confirm { title, message, .. } => rsx! {
            p { class: "eyebrow", "CONFIRMATION REQUIRED" }
            h2 { "{title}" }
            p { class: "dialog-copy", "{message}" }
            div { class: "dialog-actions",
                button {
                    class: "button secondary",
                    disabled: !owned,
                    onclick: {
                        let id = id.clone();
                        let agent_id = agent_id.clone();
                        move |_| respond_interaction(model, actions_signal, agent_id.clone(), id.clone(), ExtensionUiResponse::Confirmed { confirmed: false })
                    },
                    "No"
                }
                button {
                    class: "button primary",
                    disabled: !owned,
                    onclick: {
                        let id = id.clone();
                        let agent_id = agent_id.clone();
                        move |_| respond_interaction(model, actions_signal, agent_id.clone(), id.clone(), ExtensionUiResponse::Confirmed { confirmed: true })
                    },
                    "Yes"
                }
            }
        },
        ExtensionUiRequest::Input {
            title, placeholder, ..
        } => rsx! {
            p { class: "eyebrow", "INPUT REQUEST" }
            h2 { "{title}" }
            input {
                class: "text-input",
                placeholder: placeholder.unwrap_or_default(),
                value: "{value}",
                disabled: !owned,
                oninput: move |event| value.set(event.value()),
            }
            button {
                class: "button primary full",
                disabled: !owned,
                onclick: {
                    let id = id.clone();
                    let agent_id = agent_id.clone();
                    move |_| respond_interaction(model, actions_signal, agent_id.clone(), id.clone(), ExtensionUiResponse::Value { value: value.read().clone() })
                },
                "Submit"
            }
        },
        ExtensionUiRequest::Editor { title, .. } => rsx! {
            p { class: "eyebrow", "EDITOR REQUEST" }
            h2 { "{title}" }
            textarea {
                class: "text-area",
                rows: 10,
                value: "{value}",
                disabled: !owned,
                oninput: move |event| value.set(event.value()),
            }
            button {
                class: "button primary full",
                disabled: !owned,
                onclick: {
                    let id = id.clone();
                    let agent_id = agent_id.clone();
                    move |_| respond_interaction(model, actions_signal, agent_id.clone(), id.clone(), ExtensionUiResponse::Value { value: value.read().clone() })
                },
                "Apply"
            }
        },
        passive => rsx! {
            p { class: "eyebrow", "AGENT NOTICE" }
            h2 { "Agent interaction" }
            pre { class: "dialog-copy", "{passive:?}" }
            button {
                class: "button primary full",
                onclick: move |_| model.write().dismiss_interaction(&agent_id),
                "Dismiss"
            }
        },
    };

    rsx! {
        div { class: "dialog-backdrop",
            section { class: "interaction-dialog", role: "dialog", aria_modal: "true",
                div { class: "dialog-agent", "{interaction.envelope.agent_id}" }
                {content}
                match &interaction.lease {
                    InteractionLeaseState::Acquiring => rsx! { p { class: "lease-state", "Claiming interaction lease…" } },
                    InteractionLeaseState::Unavailable(error) => rsx! { p { class: "lease-state error", "Another client owns this prompt: {error}" } },
                    InteractionLeaseState::Owned => rsx! {},
                }
                if requires_response {
                    button {
                        class: "link-button cancel-dialog",
                        disabled: !owned,
                        onclick: {
                            let id = id.clone();
                            let agent_id = interaction.envelope.agent_id.clone();
                            move |_| respond_interaction(model, actions_signal, agent_id.clone(), id.clone(), ExtensionUiResponse::Cancelled { timed_out: Some(false) })
                        },
                        "Cancel"
                    }
                }
            }
        }
    }
}

fn respond_interaction(
    mut model: Signal<AppModel>,
    actions_signal: Signal<Option<AppActions>>,
    agent_id: AgentId,
    id: String,
    response: ExtensionUiResponse,
) {
    let Some(actions) = actions_signal.read().clone() else {
        return;
    };
    spawn(async move {
        let frame = ExtensionUiResponseFrame::Response { id, response };
        match actions
            .respond_to_interaction(agent_id.clone(), frame)
            .await
        {
            Ok(()) => model.write().dismiss_interaction(&agent_id),
            Err(error) => model.write().error = Some(error.to_string()),
        }
    });
}

fn interaction_requires_response(frame: &ExtensionUiRequestFrame) -> bool {
    matches!(
        frame,
        ExtensionUiRequestFrame::Request {
            request: ExtensionUiRequest::Select { .. }
                | ExtensionUiRequest::Confirm { .. }
                | ExtensionUiRequest::Input { .. }
                | ExtensionUiRequest::Editor { .. },
            ..
        }
    )
}

fn interaction_key(interaction: &PendingInteraction) -> String {
    let ExtensionUiRequestFrame::Request { id, .. } = &interaction.envelope.request;
    format!("{}:{id}", interaction.envelope.agent_id)
}

fn scope_badge(label: &'static str, enabled: bool) -> Element {
    rsx! { span { class: if enabled { "scope enabled" } else { "scope" }, "{label}" } }
}

fn lifecycle_label(lifecycle: &AgentLifecycle) -> String {
    match lifecycle {
        AgentLifecycle::Failed { reason } => format!("Failed — {reason}"),
        lifecycle => format!("{lifecycle:?}"),
    }
}

fn lifecycle_dot(lifecycle: &AgentLifecycle) -> &'static str {
    match lifecycle {
        AgentLifecycle::Running => "dot running",
        AgentLifecycle::Starting | AgentLifecycle::Stopping => "dot working",
        AgentLifecycle::Failed { .. } | AgentLifecycle::Interrupted => "dot failed",
        _ => "dot idle",
    }
}

fn connection_label(status: &ConnectionStatus) -> &'static str {
    match status {
        ConnectionStatus::Connected { .. } => "Connected",
        ConnectionStatus::Connecting => "Connecting",
        ConnectionStatus::Disconnected { .. } => "Reconnecting",
        ConnectionStatus::Stopped { .. } => "Stopped",
    }
}

fn connection_class(status: &ConnectionStatus) -> &'static str {
    match status {
        ConnectionStatus::Connected { .. } => "connection-state online",
        ConnectionStatus::Connecting | ConnectionStatus::Disconnected { .. } => {
            "connection-state pending"
        }
        ConnectionStatus::Stopped { .. } => "connection-state offline",
    }
}

fn transcript_class(kind: TranscriptKind) -> &'static str {
    match kind {
        TranscriptKind::User => "message user",
        TranscriptKind::Assistant => "message assistant",
        TranscriptKind::Thinking => "message thinking",
        TranscriptKind::Tool => "message tool",
        TranscriptKind::System => "message system",
    }
}

fn transcript_label(kind: TranscriptKind) -> &'static str {
    match kind {
        TranscriptKind::User => "YOU",
        TranscriptKind::Assistant => "ASSISTANT",
        TranscriptKind::Thinking => "THINKING",
        TranscriptKind::Tool => "TOOL ACTIVITY",
        TranscriptKind::System => "SYSTEM",
    }
}

fn default_device_name() -> String {
    #[cfg(target_arch = "wasm32")]
    return "Web browser".to_owned();
    #[cfg(not(target_arch = "wasm32"))]
    std::env::var("HOSTNAME").unwrap_or_else(|_| "Personal device".to_owned())
}

#[cfg(target_arch = "wasm32")]
fn unix_time_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn format_timestamp(value: Option<u64>) -> String {
    value.map_or_else(|| "never".to_owned(), |value| value.to_string())
}
