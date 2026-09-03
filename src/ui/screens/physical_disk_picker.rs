//! Physical disk picker for whole-disk passthrough.
//!
//! Lists host block devices; devices the host is using (system disk, mounted,
//! swap, LVM/LUKS/RAID) are shown greyed-out with the reason and cannot be
//! selected. Selecting a disk always goes through a destructive-action
//! confirmation dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, ConfirmAction, Screen};

use super::super::centered_rect;

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let dialog_width = 90.min(area.width.saturating_sub(4));
    let dialog_height = 24.min(area.height.saturating_sub(4));

    let dialog_area = centered_rect(dialog_width, dialog_height, area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Select Physical Disk ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Warning header
            Constraint::Length(1), // Spacer
            Constraint::Min(3),    // Device list
            Constraint::Length(2), // Help
        ])
        .split(h_chunks[1]);

    // Warning header — this screen hands host hardware to a guest
    let warning = Paragraph::new(Line::from(Span::styled(
        "⚠ The guest OS can DESTROY all data on the selected disk",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(warning, v_chunks[0]);

    if app.block_devices.is_empty() {
        let empty = Paragraph::new(
            "No physical disks found.\n\nAll disks may be in use by the host, \
             or /sys/block is unavailable.",
        )
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, v_chunks[2]);
    } else {
        let items: Vec<ListItem> = app
            .block_devices
            .iter()
            .map(|d| {
                let selectable = d.is_selectable();
                let name_style = if selectable {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let mut first_line = vec![
                    Span::styled(
                        format!("{:<28}", truncate(&d.model, 28)),
                        name_style.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:>10}  ", d.size_display()), name_style),
                    Span::styled(format!("{:<7}", d.bus.label()), name_style),
                ];
                if let Some(reason) = &d.exclusion {
                    first_line.push(Span::styled(
                        format!("  [{}]", reason.label()),
                        Style::default().fg(Color::Red),
                    ));
                } else if d.removable {
                    first_line.push(Span::styled(
                        "  [removable]",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                let mut path_str = format!("  {}", d.launch_path().display());
                if d.by_id_path.is_none() {
                    path_str.push_str("  (no stable by-id path)");
                }
                let second_line =
                    Line::from(Span::styled(path_str, Style::default().fg(Color::DarkGray)));
                ListItem::new(vec![Line::from(first_line), second_line])
            })
            .collect();

        let list = List::new(items).highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );
        let mut list_state = ListState::default();
        list_state.select(Some(app.block_device_selected));
        frame.render_stateful_widget(list, v_chunks[2], &mut list_state);
    }

    let help = Paragraph::new("[j/k] Navigate  [Enter] Select  [r] Rescan  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, v_chunks[3]);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.pop_screen();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.block_devices.is_empty()
                && app.block_device_selected + 1 < app.block_devices.len()
            {
                app.block_device_selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.block_device_selected > 0 {
                app.block_device_selected -= 1;
            }
        }
        KeyCode::Char('r') => {
            app.block_devices = crate::hardware::enumerate_block_devices().unwrap_or_default();
            app.block_device_selected = 0;
            app.set_status("Rescanned physical disks");
        }
        KeyCode::Enter => {
            let Some(device) = app.block_devices.get(app.block_device_selected) else {
                return Ok(());
            };
            if let Some(reason) = &device.exclusion {
                app.set_status(format!("Cannot select {}: {}", device.name, reason.label()));
            } else {
                app.push_screen(Screen::Confirm(ConfirmAction::UsePhysicalDisk));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Message body for the destructive confirmation dialog
pub fn confirm_message(app: &App) -> String {
    let Some(d) = app.block_devices.get(app.block_device_selected) else {
        return "No disk selected.".to_string();
    };
    format!(
        "Pass through {} ({}, {})?\n{}\n\nvm-curator will not format or copy this disk, \
         but the guest OS can overwrite ANY data on it, including partitions used by \
         other operating systems.",
        d.model,
        d.size_display(),
        d.bus.label(),
        d.launch_path().display(),
    )
}

/// Apply the confirmed selection to whoever opened the picker, then close the
/// confirm dialog and the picker.
pub fn apply_confirmed_selection(app: &mut App) {
    let Some(device) = app
        .block_devices
        .get(app.block_device_selected)
        .cloned()
        .filter(|d| d.is_selectable())
    else {
        app.pop_screen();
        return;
    };

    match app.disk_picker_context {
        crate::app::DiskPickerContext::Wizard => {
            if let Some(ref mut state) = app.wizard_state {
                state.physical_disk = Some(device);
            }
        }
        crate::app::DiskPickerContext::Management => {
            super::disk_passthrough::add_device(app, &device);
        }
    }
    app.pop_screen(); // confirm dialog
    app.pop_screen(); // picker
}
