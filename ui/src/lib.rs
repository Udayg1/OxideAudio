use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use macros::concat_strings;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Spacing},
    symbols::merge,
    widgets::{Block, Borders, Paragraph},
};
use ratatui::{
    layout::{Alignment, Rect},
    widgets::{Clear, List, ListItem},
};
use serde_json::Value;
use std::io::{Stdout, stdout};
pub enum UiMode {
    Normal,
    Search,
    Results,
}

pub struct App {
    pub status: String,
    pub search_query: String,
    pub search_results: Vec<Value>,
    pub selected: usize,
    pub queue_len: i64,
    pub paused: bool,
    pub mode: UiMode,
    pub dirty: bool,
}

pub fn setup_terminal() -> Terminal<CrosstermBackend<Stdout>> {
    enable_raw_mode().unwrap();
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    Terminal::new(CrosstermBackend::new(stdout)).unwrap()
}

pub fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();
}

pub fn draw_ui(f: &mut ratatui::Frame, app: &App, playlist: &[String]) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(f.area());
    let wid = f.area().width;
    let hei = f.area().height;
    let top_body;
    let bottom_body;
    if wid as f32 / hei as f32 > 1.5 {
        [top_body, bottom_body] = Layout::horizontal([Constraint::Fill(1); 2])
            .spacing(Spacing::Overlap(1))
            .areas(chunks[1]);
    } else {
        [top_body, bottom_body] = Layout::vertical([Constraint::Fill(1); 2])
            .spacing(Spacing::Overlap(1))
            .areas(chunks[1]);
    }
    let header = Paragraph::new("mscply — Tidal / YTM player")
        .block(Block::default().borders(Borders::ALL).title(""));
    let body = Paragraph::new(concat_strings(Vec::from([
        "Status: ",
        app.status.as_str(),
        "\nPaused: ",
        &app.paused.to_string(),
        "\nQueue: ",
        &app.queue_len.to_string(),
    ])))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Player")
            .merge_borders(merge::MergeStrategy::Exact),
    );
    let playlst = playlist.join("\n");
    let body_playlist = Paragraph::new(playlst).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Up Next")
            .merge_borders(merge::MergeStrategy::Exact),
    );
    let footer = Paragraph::new(match app.mode {
        UiMode::Normal => "[a] Add  [p] Pause/Resume  [r] Back  [s] Skip  [f] seek forward  [b] seek backward  [h] Quit",
        UiMode::Search => "Type search, Enter = search, Esc = cancel",
        UiMode::Results => "↑↓ select, Enter = add, Esc = cancel",
    })
    .block(Block::default().borders(Borders::ALL).title("Controls"));

    f.render_widget(header, chunks[0]);
    f.render_widget(body, top_body);
    f.render_widget(body_playlist, bottom_body);
    f.render_widget(footer, chunks[2]);
    if matches!(app.mode, UiMode::Search | UiMode::Results) {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);

        match app.mode {
            UiMode::Search => {
                let input = Paragraph::new(app.search_query.to_string() + "_")
                    .alignment(Alignment::Left)
                    .block(Block::default().borders(Borders::ALL).title("Search"));
                f.render_widget(input, area);
            }
            UiMode::Results => {
                let items: Vec<ListItem> = app
                    .search_results
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let title = t.get("title").and_then(Value::as_str).unwrap_or("Unknown");
                        let artist = t
                            .get("artist")
                            .and_then(|a| a.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("Unknown");

                        let prefix = if i == app.selected { "▶ " } else { "  " };
                        let time = t.get("duration").and_then(Value::as_i64).unwrap();
                        let min = time / 60;
                        let sec = time % 60;
                        let tags = t
                            .get("mediaMetadata")
                            .and_then(|v| v.get("tags"))
                            .and_then(Value::as_array);
                        let tag: &Vec<Value>;
                        let mut qual = "";
                        if !tags.is_none() {
                            tag = tags.unwrap();
                            qual = if !tag.is_empty()
                                && tag.iter().any(|v| v.as_str() == Some("HIRES_LOSSLESS"))
                            {
                                "24 bit/192kHz"
                            } else {
                                t.get("audioQuality").and_then(Value::as_str).unwrap()
                            }
                        }
                        if qual == "LOSSLESS" {
                            qual = "16 bit/44.1kHz"
                        }
                        ListItem::new(concat_strings(Vec::from([
                            prefix,
                            title,
                            "—",
                            artist,
                            " (",
                            &min.to_string(),
                            ":",
                            &sec.to_string(),
                            ", ",
                            qual,
                            ")",
                        ])))
                    })
                    .collect();

                let list =
                    List::new(items).block(Block::default().borders(Borders::ALL).title("Results"));

                f.render_widget(list, area);
            }
            _ => {}
        }
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
