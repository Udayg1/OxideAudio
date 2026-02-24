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

pub fn draw_ui(f: &mut ratatui::Frame, app: &mut App, playlist: &[Value], options: &Options) {
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
    let header = Paragraph::new("mscply — Tidal / YTM player")
        .block(Block::default().borders(Borders::ALL).title(""));
    let footer = Paragraph::new(match app.mode {
        UiMode::Normal => "[a] Add [p] Pause/Resume [r] Back [s] Skip [f] seek forward [b] seek backward [u] show/hide queue [h] Quit",
        UiMode::Search => "Type search, Enter = search, Esc = cancel",
        UiMode::Results => "↑↓ select, Enter = add, Esc = cancel",})
        .block(Block::default().borders(Borders::ALL).title("[Controls]"));
    let mut cur_time_str = String::new();
    let body;
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
    let text;
    if app.status.starts_with("Playing") && app.dur != 0 {
        text = concat_strings(Vec::from([
            app.status.as_str(),
            " (",
            &cur_time_str,
            "/",
            &dur_str,
            ")",
        ]))
    } else {
        text = app.status.to_string();
    }
    body = Paragraph::new("")
        .alignment(Alignment::Center)
        .centered()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("[Player]")
                .merge_borders(merge::MergeStrategy::Exact),
        );
    let song_body = Paragraph::new(text).centered();
    if options.queue {
        let mut top_body;
        let bottom_body;
        if wid as f32 / hei as f32 > 1.5 {
            [top_body, bottom_body] = Layout::horizontal([Constraint::Fill(0); 2])
                .spacing(Spacing::Overlap(1))
                .areas(chunks[1]);
        } else {
            [top_body, bottom_body] = Layout::vertical([Constraint::Fill(0); 2])
                .spacing(Spacing::Overlap(1))
                .areas(chunks[1]);
        }

        top_body = top_body.centered_vertically(Constraint::Fill(1));

        let [_top, bottom] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)])
            .areas(top_body);
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
        f.render_widget(body, top_body);
        f.render_widget(song_body, bottom);
        f.render_widget(body_playlist, bottom_body);
    } else {
        let _top_song;
        let bottom_song;
        [_top_song, bottom_song] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)])
            .areas(chunks[1]);
        f.render_widget(body, chunks[1]);
        f.render_widget(song_body, bottom_song);
    }
    f.render_widget(header, chunks[0]);
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
                        let mut sec_str = sec.to_string();
                        if sec < 10 {
                            sec_str = concat_strings(Vec::from(["0", &sec_str]));
                        }
                        if !tags.is_none() {
                            tag = tags.unwrap();
                            qual = if !tag.is_empty()
                                && tag.iter().any(|v| v.as_str() == Some("HIRES_LOSSLESS"))
                            {
                                "upto 24 bit/192kHz"
                            } else {
                                t.get("audioQuality").and_then(Value::as_str).unwrap()
                            }
                        }
                        if qual == "LOSSLESS" {
                            qual = "upto 16 bit/44.1kHz"
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
