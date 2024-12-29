mod line;
mod tab;

use serde::{Deserialize, Serialize};
use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::fs;
use std::io::{BufReader, BufWriter};

use tab::get_tab_to_focus;
use zellij_tile::prelude::*;

use crate::line::tab_line;
use crate::tab::tab_style;

#[derive(Debug, Default)]
pub struct LinePart {
    part: String,
    len: usize,
    tab_index: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ClientLayout {
    tab_idx: usize,
    pane: (u32, bool),
}

#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    active_tab_idx: usize,
    mode_info: ModeInfo,
    tab_line: Vec<LinePart>,
    next_session: Option<String>,
    clients: Vec<ClientInfo>,
    switch_session_event_source_pid: Option<u32>,
    current_session: String,
    pid: u32,
}

register_plugin!(State);

trait SwitchSession {
    fn try_switch_session(&mut self) -> ();
    fn dump_layout_to_cache(&self) -> ();
    fn get_session_layout_info(&self, session_name: &str) -> BTreeMap<u32, ClientLayout>;
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        set_selectable(false);
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            PermissionType::ChangeApplicationState,
            PermissionType::OpenFiles,
        ]);
        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::ModeUpdate,
            EventType::Mouse,
            EventType::SessionUpdate,
            EventType::ListClients,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;

        match event {
            Event::ModeUpdate(mode_info) => {
                if self.mode_info != mode_info {
                    should_render = true;
                }
                self.mode_info = mode_info
            }
            Event::TabUpdate(tabs) => {
                if let Some(active_tab_index) = tabs.iter().position(|t| t.active) {
                    // tabs are indexed starting from 1 so we need to add 1
                    let active_tab_idx = active_tab_index + 1;
                    if self.active_tab_idx != active_tab_idx || self.tabs != tabs {
                        should_render = true;
                    }
                    self.active_tab_idx = active_tab_idx;
                    self.tabs = tabs;
                } else {
                    eprintln!("Could not find active tab.");
                }
            }
            Event::Mouse(me) => match me {
                Mouse::LeftClick(_, col) => {
                    let tab_to_focus = get_tab_to_focus(&self.tab_line, self.active_tab_idx, col);
                    if let Some(idx) = tab_to_focus {
                        switch_tab_to(idx.try_into().unwrap());
                    }
                }
                Mouse::ScrollUp(_) => {
                    switch_tab_to(min(self.active_tab_idx + 1, self.tabs.len()) as u32);
                }
                Mouse::ScrollDown(_) => {
                    switch_tab_to(max(self.active_tab_idx.saturating_sub(1), 1) as u32);
                }
                _ => {}
            },
            Event::SessionUpdate(sessions, _) => {
                let mut all_sessions: Vec<SessionInfo> =
                    sessions.into_iter().map(|item| item).collect();
                all_sessions.sort_by(|item1, item2| item1.name.cmp(&item2.name));
                let current_session_index = all_sessions
                    .iter()
                    .position(|item| item.is_current_session)
                    .unwrap();

                self.current_session = all_sessions[current_session_index].name.clone();

                if all_sessions.len() > 1 {
                    self.next_session = all_sessions
                        .remove((current_session_index + 1) % all_sessions.len())
                        .name
                        .into();
                }
            }
            Event::PaneUpdate(panes) => {
                self.panes = panes;
            }
            Event::ListClients(clients) => {
                self.clients = clients;
                self.try_switch_session();
            }
            _ => {
                eprintln!("Got unrecognized event: {:?}", event);
            }
        };
        should_render
    }

    fn render(&mut self, _rows: usize, cols: usize) {
        if self.tabs.is_empty() {
            return;
        }

        let mut all_tabs: Vec<LinePart> = vec![];
        let mut active_tab_index = 0;
        for (index, t) in &mut self.tabs.iter().enumerate() {
            let mut tabname = t.name.clone();
            if t.active && self.mode_info.mode == InputMode::RenameTab {
                if tabname.is_empty() {
                    tabname = String::from("Enter name...");
                }
                active_tab_index = t.position;
            } else if t.active {
                active_tab_index = t.position;
            }
            let tab = tab_style(
                (index + 1).to_string() + " " + tabname.as_ref(),
                t,
                self.mode_info.style.colors,
            );
            all_tabs.push(tab);
        }
        self.tab_line = tab_line(
            self.mode_info.session_name.as_deref(),
            all_tabs,
            active_tab_index,
            cols.saturating_sub(1),
            self.mode_info.style.colors,
            self.mode_info.capabilities,
            self.mode_info.mode,
        );
        let output = self
            .tab_line
            .iter()
            .fold(String::new(), |output, part| output + &part.part);
        let background = match self.mode_info.style.colors.theme_hue {
            ThemeHue::Dark => self.mode_info.style.colors.black,
            ThemeHue::Light => self.mode_info.style.colors.white,
        };
        match background {
            PaletteColor::Rgb((r, g, b)) => {
                print!("{}\u{1b}[48;2;{};{};{}m\u{1b}[0K", output, r, g, b);
            }
            PaletteColor::EightBit(color) => {
                print!("{}\u{1b}[48;5;{}m\u{1b}[0K", output, color);
            }
        };
    }

    fn pipe(&mut self, pipe_msg: PipeMessage) -> bool {
        if pipe_msg.name == "switch_session" {
            self.switch_session_event_source_pid = match pipe_msg.source {
                PipeSource::Keybind {
                    source_client_id: _,
                    source_pid,
                } => Some(source_pid),
                _ => None,
            };
            list_clients();
        }
        true
    }
}

impl SwitchSession for State {
    fn try_switch_session(&mut self) -> () {
        if self.current_session.is_empty() {
            return ();
        }

        let plugin_pid = self
            .clients
            .iter()
            .find(|client_info| client_info.is_current_client)
            .map(|v| v.client_pid);

        if plugin_pid.is_none() {
            return ();
        }

        self.pid = plugin_pid.unwrap();
        if self.switch_session_event_source_pid.is_some()
            && self.pid == self.switch_session_event_source_pid.unwrap()
        {
            if self.next_session.is_some() {
                self.dump_layout_to_cache();

                let next_session = self.next_session.as_deref().unwrap();
                match self
                    .get_session_layout_info(&next_session)
                    .remove(&self.pid)
                {
                    Some(layout) => {
                        switch_session_with_focus(
                            next_session,
                            layout.tab_idx.into(),
                            layout.pane.into(),
                        );
                    }
                    None => {
                        switch_session(self.next_session.as_deref());
                    }
                }
            }
        }

        self.switch_session_event_source_pid = None;
    }

    fn dump_layout_to_cache(&self) -> () {
        let focused_tab_idx = get_focused_tab(&self.tabs).map(|tab| tab.position);
        if focused_tab_idx.is_none() {
            return ();
        }

        let focused_pane = get_focused_pane(focused_tab_idx.unwrap(), &self.panes);
        if focused_pane.is_none() {
            return ();
        }

        let layout = ClientLayout {
            tab_idx: focused_tab_idx.unwrap(),
            pane: (
                focused_pane.as_ref().unwrap().id,
                focused_pane.unwrap().is_plugin,
            ),
        };

        let mut layout_info = self.get_session_layout_info(&self.current_session);
        layout_info.insert(self.pid, layout);

        serde_json::to_writer_pretty(
            BufWriter::new(
                fs::File::create(format!("/tmp/{0}.json", self.current_session))
                    .expect("could not open file"),
            ),
            &layout_info,
        )
        .unwrap();
    }

    fn get_session_layout_info(&self, session_name: &str) -> BTreeMap<u32, ClientLayout> {
        let file = fs::File::open(format!("/tmp/{0}.json", session_name));
        if file.is_ok() {
            let reader = BufReader::new(file.unwrap());
            match serde_json::from_reader(reader) {
                Ok(val) => val,
                Err(_) => BTreeMap::new(),
            }
        } else {
            BTreeMap::new()
        }
    }
}
