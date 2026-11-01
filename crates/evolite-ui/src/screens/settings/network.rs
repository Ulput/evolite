//! Network: the network, its nodes and the sync as the core reports it; "Own node".

use super::parts::{Control, group, heading};
use super::{NodeForm, SettingsState};
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, dialog, field_label, kv_row, notice,
    segmented, text_field,
};
use crate::context::Ctx;
use crate::screens::txt;
use crate::theme::Weight;
use egui::Ui;
use evolite_api::{Command, CoreError};

pub(super) fn network(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let sync = &cx.snap.sync;
    let core_nodes = format!(tr!("{} nodes"), sync.core_peers);
    let core_line = match &sync.core_node {
        Some(node) => format!(tr!("Only own node {node}"), node = node),
        None => format!(
            tr!("{} of 8 connected · found via DNS seeds"),
            sync.core_peers
        ),
    };
    let platform_nodes = format!(tr!("{} nodes"), sync.platform_nodes);
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("Network and nodes"),
        Some(tr!(
            "evolite picks the nodes itself and checks everything they send. There is usually nothing to change here."
        )),
    );
    // The network is chosen at the start (`--network mainnet`), never here: the control shows
    // which one this run is on, and the other cannot be picked.
    let mainnet = cx.snap.network == evolite_api::Network::Mainnet;
    let mut net = usize::from(mainnet);
    let (net_line, off) = if mainnet {
        (
            // No command lines here: a person does not start programs with flags.
            tr!(
                "Mainnet means real DASH. To go back to testnet, close evolite and open it as usual."
            ),
            [Some(tr!("Close evolite and open it as usual")), None],
        )
    } else {
        (
            tr!(
                "Testnet is for trying things out; its coins are worth nothing. Mainnet, with real DASH, is not switched on in this test build."
            ),
            [None, Some(tr!("Not in this test build"))],
        )
    };
    let clicked = group(
        ui,
        a,
        tr!("Network"),
        vec![
            (
                tr!("Network"),
                net_line,
                Control::SegmentedWithOff(&["Testnet", "Mainnet"], &mut net, &off),
            ),
            (
                tr!("Core nodes"),
                &core_line,
                Control::Value(&core_nodes, false),
            ),
            (
                tr!("Platform (DAPI)"),
                tr!("Answers are verified with GroveDB proofs"),
                Control::Value(&platform_nodes, false),
            ),
        ],
    );
    // the nodes behind the counts.
    match clicked {
        Some(1) => state.nodes = Some(super::NodesTab::Core),
        Some(2) => state.nodes = Some(super::NodesTab::Platform),
        _ => {}
    }
    // The node for the next start, as the core keeps it: what this run uses unless changed.
    let saved = match &sync.core_node_next {
        Some(next) if next.is_empty() => None,
        Some(next) => Some(next.clone()),
        None => sync.core_node.clone(),
    };
    let was_on = saved.is_some() || state.node.is_some();
    let mut own = was_on;
    let own_line = match &saved {
        Some(node) => format!(
            tr!(
                "Only {node}. Headers, ChainLock and InstantLock are checked as always; Platform uses its own nodes."
            ),
            node = node
        ),
        None => tr!("Connect only to the given Core node, e.g. your own dashd.").into(),
    };
    group(
        ui,
        a,
        tr!("Verification"),
        vec![
            (
                tr!("ChainLock and InstantLock"),
                tr!("Quorums' BLS signatures are always verified. This cannot be turned off."),
                Control::Locked,
            ),
            (tr!("Own node"), &own_line, Control::Toggle(&mut own)),
            (
                tr!("Proxy (SOCKS5)"),
                tr!(
                    "Not yet: dash-spv connects to Core nodes only directly, so only part of the traffic would go through a proxy — the IP would still be visible"
                ),
                Control::Text(tr!("unavailable"), false),
            ),
        ],
    );
    if own && !was_on {
        state.node = Some(NodeForm::default());
    } else if !own && was_on {
        state.node = None;
        if saved.is_some() {
            cx.send(Command::CoreNode { address: None });
        }
    }
    if let Some(form) = &mut state.node {
        node_form(ui, a, cx, form, saved.as_deref());
        if form.result == Some(Ok(())) {
            state.node = None;
        }
    }
    if let Some(next) = &sync.core_node_next {
        let line = if next.is_empty() {
            tr!("Own node is off. After a restart evolite finds nodes through DNS seeds again.")
                .to_string()
        } else {
            format!(
                tr!("Node {next} saved. evolite connects to it after a restart."),
                next = next
            )
        };
        notice(ui, a, "info", p.accent, &line);
        if button(ui, a, tr!("Restart evolite"), ButtonKind::Secondary, None).clicked() {
            state.restart = true;
        }
    }
    let disk_line = match sync.disk_bytes {
        Some(bytes) => format!(
            tr!("Headers, block filters and history · {}"),
            crate::text::bytes(bytes)
        ),
        None => tr!("Headers, block filters and history · counting…").into(),
    };
    let asked = group(
        ui,
        a,
        tr!("Data"),
        vec![(
            tr!("Disk space"),
            &disk_line,
            Control::Button(tr!("Sync again"), ButtonKind::Secondary),
        )],
    );
    if asked.is_some() {
        state.resync = Some(Default::default());
    }
}

/// The address of the node and "Save"; the core checks that it resolves.
fn node_form(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut NodeForm, saved: Option<&str>) {
    let p = a.p;
    let mainnet_port = cx.snap.network == evolite_api::Network::Mainnet;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        field_label(ui, &p, tr!("Core node address"));
        let edited = text_field(
            ui,
            &p,
            "core-node",
            &mut form.address,
            saved.unwrap_or(if mainnet_port { "192.168.1.10:9999" } else { "192.168.1.10:19999" }),
            FieldKind::Text,
            |_| {},
        )
        .changed();
        if edited {
            form.result = None;
        }
        let line = match &form.result {
            Some(Err(CoreError::NotAvailable(why) | CoreError::Failed(why))) => {
                Some((why.clone(), p.danger))
            }
            Some(Err(e)) => Some((crate::text::failed(tr!("Was not saved"), e), p.danger)),
            _ => Some((
                if mainnet_port {
                    tr!("An IP or a name; port 9999 if none is given. evolite cannot connect to other nodes: if this one is down, there is no sync.")
                } else {
                    tr!("An IP or a name; port 19999 if none is given. evolite cannot connect to other nodes: if this one is down, there is no sync.")
                }
                .into(),
                p.muted,
            )),
        };
        if let Some((line, color)) = line {
            ui.add(egui::Label::new(txt(&line, 12.0, Weight::Regular, color)).wrap());
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let ready = !form.address.trim().is_empty() && !form.waiting;
            let label = if form.waiting {
                tr!("Checking…")
            } else {
                tr!("Save")
            };
            if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked() {
                cx.send(Command::CoreNode {
                    address: Some(form.address.trim().to_owned()),
                });
                form.waiting = true;
            }
        });
    });
}

/// · Nodes: the Core nodes connected now and the Platform (DAPI) nodes the SDK may ask,
/// each with what it is. Only to look at: evolite picks them, checks what they send, and
/// "Own node" is the way to choose. Returns true when closed. Also over the sync after a
/// restore ("About the nodes").
pub fn nodes_dialog(ui: &mut Ui, a: &mut Assets, cx: &Ctx, tab: &mut super::NodesTab) -> bool {
    use super::NodesTab;
    let sync = &cx.snap.sync;
    let now = cx.now.timestamp();
    let p = a.p;
    dialog(
        ui,
        a,
        "nodes-dialog",
        "peers",
        tr!("Nodes"),
        560.0,
        |ui, a| {
            let mut close = false;
            let mut i = usize::from(*tab == NodesTab::Platform);
            segmented(ui, &p, &["Core", "Platform (DAPI)"], &mut i, false);
            *tab = if i == 0 {
                NodesTab::Core
            } else {
                NodesTab::Platform
            };
            let (lead, nodes) = match tab {
                NodesTab::Core => (
                    match &sync.core_node {
                        Some(node) => format!(tr!("Only own node {node}."), node = node),
                        None => format!(
                            tr!(
                                "{} of 8 connected. Nodes are not trusted: headers, ChainLock and InstantLock are checked here."
                            ),
                            sync.core_nodes.len()
                        ),
                    },
                    &sync.core_nodes,
                ),
                NodesTab::Platform => (
                    format!(
                        tr!(
                            "{} of {} answer. Answers are checked with proofs against quorum keys from our own masternode list; a node with a bad answer waits its turn."
                        ),
                        sync.dapi_nodes
                            .iter()
                            .filter(|n| n.banned_until.is_none_or(|t| t <= now))
                            .count(),
                        sync.dapi_nodes.len()
                    ),
                    &sync.dapi_nodes,
                ),
            };
            ui.add(egui::Label::new(txt(&lead, 13.0, Weight::Regular, p.muted)).wrap());
            if nodes.is_empty() {
                ui.label(txt(
                    tr!("Looking for nodes…"),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for node in nodes {
                        let state = match node.banned_until {
                            Some(t) if t > now => {
                                format!(tr!("waits until {}"), clock(t))
                            }
                            _ => role(node.role).to_owned(),
                        };
                        kv_row(
                            ui,
                            &p,
                            &node.address,
                            &state,
                            crate::theme::mono(12.0, Weight::Regular),
                        );
                        ui.add_space(6.0);
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, a, tr!("Close"), ButtonKind::Secondary, None).clicked() {
                    close = true;
                }
            });
            close
        },
    )
}

/// What a node is, as the masternode list says.
pub(super) fn role(role: evolite_api::NodeRole) -> &'static str {
    match role {
        evolite_api::NodeRole::Unknown => tr!("the masternode list is still loading"),
        evolite_api::NodeRole::Regular => tr!("regular node"),
        evolite_api::NodeRole::Masternode => tr!("masternode"),
        evolite_api::NodeRole::Evonode => tr!("evonode"),
    }
}

/// "23:58": a time later today, local.
fn clock(t: evolite_api::Timestamp) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_is_said_in_words() {
        use evolite_api::NodeRole::*;
        let words: Vec<_> = [Unknown, Regular, Masternode, Evonode].map(role).to_vec();
        assert_eq!(words.len(), 4);
        assert!(words.iter().all(|w| !w.is_empty()));
        assert_eq!(role(Evonode), tr!("evonode"));
    }
}
