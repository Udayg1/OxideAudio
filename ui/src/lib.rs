use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use macros::concat_strings;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    symbols::merge,
    widgets::{Block, Borders, Paragraph},
};
use ratatui::{
    layout::Alignment,
    widgets::{Clear, List, ListItem},
};
use serde_json::Value;
use std::io::{Stdout, stdout};

pub enum UiMode {
    Normal,
    Search,
    Suggestions,
    Results,
}

pub struct Options {
    pub queue: bool,
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
    pub cur_time: i64,
    pub dur: i64,
    pub image: String,
    pub track_format: String,
    pub sample_rate: i64,
    pub channel_count: i64,
    pub bitrate: i64,
    pub msg: String,
}

pub fn setup_terminal() -> Terminal<CrosstermBackend<Stdout>> {
    enable_raw_mode().unwrap();
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )
    .unwrap();
    Terminal::new(CrosstermBackend::new(stdout)).unwrap()
}

pub fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();
}

pub fn draw_ui(f: &mut ratatui::Frame, app: &mut App, playlist: &[Value], _options: &Options) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Length(3),
        ])
        .split(f.area());
    let footer = Paragraph::new(match app.mode {
        UiMode::Normal => {
            "[a] Add [p] Pause/Resume [r] Back [s] Skip [f] seek forward [b] seek backward [h] Quit"
        }
        UiMode::Search => "Type search, Enter = search, Esc = cancel",
        UiMode::Results => "↑↓ select, Enter = add, Esc = cancel",
        UiMode::Suggestions => "↑↓ select, Enter = add, Esc = cancel",
    })
    .block(Block::default().borders(Borders::ALL).title("[ Controls ]"));
    let mut cur_time_str = String::new();
    let cur_min = app.cur_time / 60;
    if cur_min > 60 {
        cur_time_str += &(cur_min / 60).to_string();
        cur_time_str += ":";
    }
    cur_time_str += &cur_min.to_string();
    cur_time_str += ":";
    if app.cur_time % 60 < 10 {
        cur_time_str += "0";
        cur_time_str += &(app.cur_time % 60).to_string();
    } else {
        cur_time_str += &(app.cur_time % 60).to_string();
    };
    let mut dur_str = String::new();
    let cur_min = app.dur / 60;
    if cur_min > 60 {
        dur_str += &(cur_min / 60).to_string();
        dur_str += ":";
    }
    dur_str += &cur_min.to_string();
    dur_str += ":";
    if (app.dur % 60) < 10 {
        dur_str += "0";
        dur_str += &(app.dur % 60).to_string();
    } else {
        dur_str += &(app.dur % 60).to_string();
    };
    let mut text = String::new();
    text += " >> ";
    if !app.status.starts_with("Nothing") {
        if !app.paused {
            text += "▶︎ "
        } else {
            text += "⏸ "
        }
        text += &app.status;
        text += "\n";
        if app.dur != 0 {
            text += &concat_strings(Vec::from([
                "[",
                &cur_time_str,
                "] [",
                &dur_str,
                "] • ",
                &app.track_format,
                " • ",
                &(app.sample_rate as f64 / 1000.0).to_string(),
                "kHz • ",
                &(app.bitrate / 1024).to_string(),
                "kbps • ",
                if app.channel_count == 2 {
                    "STEREO"
                } else {
                    "MONO"
                },
            ]));
        }
    } else {
        text += &app.status;
    }
    if !app.msg.is_empty() {
        text += "\nError - ";
        text += &app.msg;
    }
    let header = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("[Playing...   ]"),
    );

    let mut playlst = String::new();
    for i in playlist {
        let name = i.get("name").and_then(Value::as_str).unwrap();
        playlst += name;
        playlst += "\n";
    }
    playlst = playlst.trim().to_string();
    let body_playlist = Paragraph::new(playlst).block(
        Block::default()
            .borders(Borders::ALL)
            .title(concat_strings(Vec::from([
                "[Up Next - ",
                &app.queue_len.to_string(),
                "]",
            ])))
            .merge_borders(merge::MergeStrategy::Exact),
    );
    f.render_widget(body_playlist, chunks[1]);
    f.render_widget(header, chunks[0]);
    f.render_widget(footer, chunks[2]);
    if matches!(
        app.mode,
        UiMode::Search | UiMode::Results | UiMode::Suggestions
    ) {
        let area = chunks[1];
        f.render_widget(Clear, area);

        match app.mode {
            UiMode::Search => {
                let input = Paragraph::new(app.search_query.to_string() + "_")
                    .alignment(Alignment::Left)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("[ Search - Enter the query ]"),
                    );
                f.render_widget(input, area);
            }
            UiMode::Results => {
                let items: Vec<ListItem> = app
                    .search_results
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let title = t.get("title").and_then(Value::as_str).unwrap_or("Unknown");
                        let artist = t.get("artist").and_then(Value::as_str).unwrap_or("Unknown");
                        let prefix = if i == app.selected { "▶ " } else { "  " };
                        let time = t.get("duration").and_then(Value::as_i64).unwrap_or(0);
                        let min = time / 60;
                        let sec = time % 60;
                        let mut sec_str = sec.to_string();
                        if sec < 10 {
                            sec_str = concat_strings(Vec::from(["0", &sec_str]));
                        }
                        ListItem::new(concat_strings(Vec::from([
                            prefix,
                            title,
                            " — ",
                            artist,
                            " (",
                            &min.to_string(),
                            ":",
                            &sec_str,
                            ")",
                        ])))
                    })
                    .collect();

                let list = List::new(items)
                    .block(Block::default().borders(Borders::ALL).title("[ Results ]"));
                f.render_widget(list, area);
            }
            UiMode::Suggestions => {
                let items: Vec<ListItem> = app
                    .search_results
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let title = t.get("title").and_then(Value::as_str).unwrap_or("Unknown");
                        let artist = t.get("artist").and_then(Value::as_str).unwrap_or("Unknown");
                        let prefix = if i == app.selected { "▶ " } else { "  " };
                        ListItem::new(concat_strings(Vec::from([prefix, title, " — ", artist])))
                    })
                    .collect();

                let list = List::new(items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("[ Suggestions ]"),
                );
                f.render_widget(list, area);
            }
            _ => {}
        }
    }
}
