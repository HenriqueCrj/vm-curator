//! Passthrough Disks Screen
//!
//! Attaches whole physical disks (NVMe/SATA/USB block devices) to an existing
//! VM via a marker-delimited section in launch.sh, mirroring the USB and
//! shared-folders mechanisms. Each disk can carry a firmware bootindex.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    prelude::*,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, ConfirmAction, DiskPickerContext, Screen, UnsavedKind};
use crate::hardware::BlockDevice;
use crate::vm::DiskPassthrough;

/// Render the passthrough disks screen
pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();

    let dialog_width = 84.min(area.width.saturating_sub(4));
    let dialog_height = 22.min(area.height.saturating_sub(4));

    let dialog_area = centered_rect(dialog_width, dialog_height, area);
    frame.render_widget(Clear, dialog_area);

    let title = format!(
        " Passthrough Disks ({} configured) ",
        app.disk_passthrough.len()
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
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
            Constraint::Length(2), // Warning banner
            Constraint::Length(1), // Spacer
            Constraint::Min(3),    // Disk list
            Constraint::Length(3), // Notes
            Constraint::Length(2), // Help text
        ])
        .split(h_chunks[1]);

    // Warning banner
    let warning = Paragraph::new(vec![
        Line::from(Span::styled(
            "⚠ These disks are passed through raw: the guest OS can destroy",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "their contents. Only attach disks the host is not using.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
    ]);
    frame.render_widget(warning, v_chunks[0]);

    // Disk list
    if app.disk_passthrough.is_empty() {
        let empty_msg = Paragraph::new("No passthrough disks configured.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, v_chunks[2]);
    } else {
        let items: Vec<ListItem> = app
            .disk_passthrough
            .iter()
            .enumerate()
            .map(|(i, disk)| {
                let style = if i == app.disk_passthrough_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let bootindex = match disk.bootindex {
                    Some(n) => format!("  [boot #{n}]"),
                    None => String::new(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {}. ", i + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(&disk.path, style),
                    Span::styled(bootindex, Style::default().fg(Color::Green)),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(app.disk_passthrough_selected));
        let list = List::new(items).highlight_symbol("> ");
        frame.render_stateful_widget(list, v_chunks[2], &mut state);
    }

    // Notes
    let notes = Paragraph::new(
        "Disks are attached as virtio-blk (raw). Boot index sets firmware boot\n\
         priority (lower boots first); the firmware boot menu is enabled while\n\
         any passthrough disk is configured.",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(notes, v_chunks[3]);

    // Help text
    let help = Paragraph::new("[a] Add  [d] Remove  [b] Boot index  [s] Save  [Esc] Back")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help, v_chunks[4]);
}

/// Handle key input for the passthrough disks screen
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            if app.disk_passthrough_dirty() {
                app.push_screen(Screen::Confirm(ConfirmAction::UnsavedChanges(
                    UnsavedKind::DiskPassthrough,
                )));
            } else {
                app.pop_screen();
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.disk_passthrough.is_empty()
                && app.disk_passthrough_selected < app.disk_passthrough.len() - 1
            {
                app.disk_passthrough_selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.disk_passthrough_selected > 0 {
                app.disk_passthrough_selected -= 1;
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.open_physical_disk_picker(DiskPickerContext::Management);
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            if !app.disk_passthrough.is_empty()
                && app.disk_passthrough_selected < app.disk_passthrough.len()
            {
                app.disk_passthrough.remove(app.disk_passthrough_selected);
                if app.disk_passthrough_selected >= app.disk_passthrough.len()
                    && app.disk_passthrough_selected > 0
                {
                    app.disk_passthrough_selected -= 1;
                }
            }
        }
        KeyCode::Char('b') | KeyCode::Char('B') => {
            if let Some(disk) = app.disk_passthrough.get_mut(app.disk_passthrough_selected) {
                // Cycle None -> 0 -> 1 -> 2 -> 3 -> None
                disk.bootindex = match disk.bootindex {
                    None => Some(0),
                    Some(n) if n < 3 => Some(n + 1),
                    Some(_) => None,
                };
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            save_selection_and_report(app);
        }
        _ => {}
    }
    Ok(())
}

/// Append a device chosen in the physical disk picker (Management context).
pub(crate) fn add_device(app: &mut App, device: &BlockDevice) {
    let path = device.launch_path().display().to_string();
    if app.disk_passthrough.iter().any(|d| d.path == path) {
        app.set_status(format!("{} is already configured", path));
        return;
    }
    app.disk_passthrough.push(DiskPassthrough {
        path,
        bootindex: None,
    });
    app.disk_passthrough_selected = app.disk_passthrough.len() - 1;
}

/// Save the passthrough disk list to launch.sh and report via the status line.
/// Shared by the `s` key and the unsaved-changes prompt.
pub(crate) fn save_selection_and_report(app: &mut App) {
    let save_result = app.selected_vm().map(|vm| {
        (
            crate::vm::save_disk_passthrough(vm, &app.disk_passthrough),
            app.disk_passthrough.len(),
        )
    });

    if let Some((result, count)) = save_result {
        match result {
            Ok(()) => {
                app.reload_selected_vm_script();

                let mut status_msg = if count > 0 {
                    format!("Saved {} passthrough disk(s) to launch.sh", count)
                } else {
                    "Cleared passthrough disks from launch.sh".to_string()
                };

                // Keep single-GPU scripts' copied QEMU command in sync.
                if let Some(vm) = app.selected_vm() {
                    let regen_result = if let Some(config) = app.single_gpu_config.as_ref() {
                        crate::vm::single_gpu_scripts::regenerate_if_exists(vm, config)
                    } else {
                        crate::vm::single_gpu_scripts::regenerate_from_saved_config(vm)
                    };

                    match regen_result {
                        Ok(true) => {
                            status_msg.push_str("; single-GPU scripts regenerated");
                        }
                        Ok(false) => {}
                        Err(e) => {
                            status_msg.push_str(&format!(
                                "; warning: failed to regenerate single-GPU scripts: {}",
                                e
                            ));
                        }
                    }
                }
                app.snapshot_disk_passthrough_baseline();
                app.set_status(status_msg);
            }
            Err(e) => {
                app.set_status(format!("Error saving passthrough disks: {}", e));
            }
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
