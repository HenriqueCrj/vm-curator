//! Virtual disk picker used when a VM has more than one system disk.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Screen};

use super::super::centered_rect;

pub fn render(app: &App, frame: &mut Frame) {
    let Screen::ResizeDiskPicker {
        disk_paths,
        selected,
        ..
    } = &app.screen
    else {
        return;
    };

    let area = frame.area();
    let dialog_width = 90.min(area.width.saturating_sub(4));
    let dialog_height = (disk_paths.len() as u16 + 8)
        .min(22)
        .min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Select Disk to Resize ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);

    let instructions = Paragraph::new(
        "This VM has multiple virtual system disks. Select the image whose capacity you want to increase.",
    )
    .style(Style::default().fg(Color::Gray))
    .wrap(Wrap { trim: true });
    frame.render_widget(instructions, chunks[0]);

    let items = disk_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("Disk {}  ", index + 1),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(path.display().to_string()),
            ]))
        })
        .collect::<Vec<_>>();
    let list = List::new(items).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(*selected));
    frame.render_stateful_widget(list, chunks[1], &mut state);

    let help = Paragraph::new("[j/k or ↑/↓] Navigate  [Enter] Select  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}
