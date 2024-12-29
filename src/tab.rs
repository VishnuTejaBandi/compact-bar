use crate::LinePart;
use ansi_term::ANSIStrings;
use unicode_width::UnicodeWidthStr;
use zellij_tile::prelude::*;
use zellij_tile_utils::style;

pub fn render_tab(text: String, tab: &TabInfo, palette: Palette) -> LinePart {
    let tab_text_len = text.width() + 2; // + 2 for padding

    let tab_styled_text = if tab.active {
        style!(palette.black, palette.yellow).paint(format!(" {} ", text))
    } else {
        style!(palette.fg, palette.bg).paint(format!(" {} ", text))
    };

    let tab_styled_text = ANSIStrings(&[tab_styled_text]).to_string();

    LinePart {
        part: tab_styled_text,
        len: tab_text_len,
        tab_index: Some(tab.position),
    }
}

pub fn tab_style(mut tabname: String, tab: &TabInfo, palette: Palette) -> LinePart {
    if tab.is_sync_panes_active {
        tabname.push_str(" (Sync)");
    }

    render_tab(tabname, tab, palette)
}

pub(crate) fn get_tab_to_focus(
    tab_line: &[LinePart],
    active_tab_idx: usize,
    mouse_click_col: usize,
) -> Option<usize> {
    let clicked_line_part = get_clicked_line_part(tab_line, mouse_click_col)?;
    let clicked_tab_idx = clicked_line_part.tab_index?;
    // tabs are indexed starting from 1 so we need to add 1
    let clicked_tab_idx = clicked_tab_idx + 1;
    if clicked_tab_idx != active_tab_idx {
        return Some(clicked_tab_idx);
    }
    None
}

pub(crate) fn get_clicked_line_part(
    tab_line: &[LinePart],
    mouse_click_col: usize,
) -> Option<&LinePart> {
    let mut len = 0;
    for tab_line_part in tab_line {
        if mouse_click_col >= len && mouse_click_col < len + tab_line_part.len {
            return Some(tab_line_part);
        }
        len += tab_line_part.len;
    }
    None
}
