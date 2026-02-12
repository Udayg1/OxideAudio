use crossterm::event::{Event, KeyCode};
use libmpv2::{
    Format, Mpv,
    events::{Event as eve, PropertyData},
};
use macros::*;
use network::*;
use player::*;
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio::time;
use ui::*;

fn advance_playback(
    mpv: &mut Mpv,
    urls: &[String],
    names: &[String],
    current: &mut usize,
    app: &mut App,
) {
    if *current + 1 >= urls.len() {
        app.status = "Nothing is playing".to_string();
        app.queue_len = 0;
        app.dirty = true;
        *current = urls.len();
        if !mpv.get_property::<bool>("idle-active").unwrap() {
            mpv.command("seek", &["100", "absolute-percent"]).unwrap();
        }
        return;
    }

    *current += 1;
    app.queue_len = (urls.len() - *current - 1) as i64;
    app.status = concat_strings(Vec::from(["Playing ", &names[*current]]));

    if urls[*current].starts_with("<?xml") {
        queue_mpd_song(mpv, &urls[*current]);
    } else {
        queue_song(mpv, &urls[*current]);
    }

    app.dirty = true;
}
fn rewind_playback(
    mpv: &mut Mpv,
    urls: &[String],
    names: &[String],
    current: &mut usize,
    app: &mut App,
) {
    if *current > 0 && urls.len() > 0 {
        if urls[*current - 1].starts_with("<xml") {
            queue_mpd_song(mpv, &urls[*current - 1]);
        } else {
            queue_song(mpv, &urls[*current - 1]);
        }
        *current -= 1;
        app.status = concat_strings(Vec::from(["Playing ", &names[*current]]));
        app.queue_len = (urls.len() - *current - 1) as i64;
        app.dirty = true;
    }
}

#[tokio::main]
async fn main() {
    let mut qual = "LOSSLESS";
    let args: Vec<String> = env::args().collect();
    let mut save = false;
    if args.len() > 1 {
        let path = concat_strings(Vec::from([
            &env::var("HOME").unwrap(),
            "/.local/share/mscply/songs/",
            "i",
        ]))
        .rsplit_once("/")
        .unwrap()
        .0
        .to_string();
        for i in &args[1..] {
            if i == "-c" {
                match fs::remove_dir_all(&path) {
                    Ok(_v) => {
                        println!("Removed cache files");
                    }
                    Err(e) => {
                        eprintln!("Error removing direcotry:  {}, {}", path, e);
                    }
                }
                return;
            } else if i == "-s" {
                save = true;
            } else if i == "-n" {
                qual = "HIGH";
            } else if i == "-l" {
                qual = "LOW"
            }
        }
    }
    PREF_QUAL
        .set(qual.to_string())
        .expect("Quality already specified");
    set_url();
    SAVE_DATA.set(save).expect("Already set");
    let (tx, rx): (Sender<QueueItem>, Receiver<QueueItem>) = mpsc::channel();
    // let log_file_location = concat_strings(Vec::from([
    //     &env::var("HOME").unwrap(),
    //     "/.local/share/mscply/mpv.log",
    // ]));
    let mut mpv = match Mpv::with_initializer(|_init| {
        // init.set_option("msg-level", "all=trace").unwrap();
        Ok(())
    }) {
        Ok(player) => player,
        Err(e) => {
            eprintln!("Failed to start MPV: {}", e);
            return;
        }
    };
    mpv.set_property(
        "demuxer-lavf-o",
        "protocol_whitelist=[file,https,http,tls,tcp,crypto,data]",
    )
    .unwrap();
    // mpv.set_property("msg-level", "all=debug").unwrap();
    // mpv.set_property("log-file", log_file_location.to_string()).unwrap();
    let mut terminal = setup_terminal();
    let mut names: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();
    let mut current = 0;
    let mut app = App {
        status: "Nothing is playing".into(),
        search_query: String::new(),
        search_results: Vec::new(),
        selected: 0,
        queue_len: 0,
        paused: false,
        mode: UiMode::Normal,
        dirty: true,
    };
    mpv.observe_property("idle-active", Format::Flag, 2)
        .unwrap();
    let mut last_mode_switch = Instant::now() - Duration::from_secs(1);
    let skip_every = Duration::from_millis(800);
    let mut auto_started = false;
    let mut skipped = false;
    let mut last = terminal.size().unwrap();
    // let mut logfile = OpenOptions::new()
    //     .read(true)
    //     .write(true)
    //     .create(true)
    //     .open(log_file_location)
    //     .unwrap();
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        if !IS_RUNNING.load(Ordering::SeqCst) && current + 1 == names.len() {
            if names.len() > 1 {
                spawn_recommendation_worker(names.last().unwrap().to_string(), tx.clone());
            }
        }
        if let Some(event) = mpv.wait_event(0.0) {
            match event {
                Ok(e) => match e {
                    // eve::LogMessage {
                    //     prefix: _,
                    //     level: _,
                    //     text,
                    //     log_level: _,
                    // } => {
                    //     logfile.write_all(text.as_bytes()).unwrap();
                    // }
                    eve::EndFile(_) => {
                        if !skipped && !auto_started {
                            auto_started = true;
                            advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                        }
                        skipped = false;
                    }

                    eve::PropertyChange {
                        reply_userdata: 2,
                        change,
                        ..
                    } => {
                        if let PropertyData::Flag(true) = change {
                            if !auto_started && current < urls.len() && !urls.is_empty() {
                                auto_started = true;
                                advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if mpv.get_property::<bool>("idle-active").unwrap() && urls.len() >= current + 1 {
            if last_mode_switch.elapsed() >= skip_every {
                advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                last_mode_switch = Instant::now();
                auto_started = true;
            }
        }
        if app.dirty {
            if current != names.len() {
                terminal
                    .draw(|f| draw_ui(f, &app, &names[current + 1..]))
                    .unwrap();
            } else {
                terminal
                    .draw(|f| draw_ui(f, &app, &names[current..]))
                    .unwrap();
            }
            app.dirty = false;
        }
        while let Ok(item) = rx.try_recv() {
            match item {
                QueueItem::Url(url) => {
                    if !names.contains(&url[1]) {
                        urls.push(url[0].clone());
                        names.push(url[1].clone());
                        app.queue_len += 1;
                    }
                }
                QueueItem::Mpd(mpd) => {
                    if !names.contains(&mpd[1]) {
                        urls.push(mpd[0].clone());
                        names.push(mpd[1].clone());
                        app.queue_len += 1;
                    }
                }
            }
            app.dirty = true;
        }
        if crossterm::event::poll(time::Duration::from_millis(10)).unwrap() {
            while crossterm::event::poll(Duration::from_millis(0)).unwrap() {
                let event = Some(crossterm::event::read().unwrap());

                match event.unwrap() {
                    Event::Key(key) => match app.mode {
                        UiMode::Normal => match key.code {
                            KeyCode::Char('h') => {
                                app.dirty = true;
                                SHUTDOWN.store(true, Ordering::SeqCst);
                                break;
                            }
                            KeyCode::Char('p') => {
                                if !app.paused {
                                    mpv.set_property("pause", true).unwrap();
                                    app.paused = true;
                                    app.dirty = true;
                                } else if app.paused {
                                    mpv.set_property("pause", false).unwrap();
                                    app.paused = false;
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Char('r') => {
                                skipped = true;
                                rewind_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                            }
                            KeyCode::Char('s') => {
                                skipped = true;
                                advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                            }
                            KeyCode::Char('a') => {
                                app.dirty = true;
                                app.search_query.clear();
                                app.mode = UiMode::Search;
                            }
                            KeyCode::Char('f') => {
                                if !mpv.get_property::<bool>("idle-active").unwrap() {
                                    match mpv.command("seek", &["5", "relative"]) {
                                        Ok(()) => {}
                                        _ => {}
                                    }
                                }
                            }
                            KeyCode::Char('b') => {
                                if !mpv.get_property::<bool>("idle-active").unwrap() {
                                    match mpv.command("seek", &["-5", "relative"]) {
                                        Ok(()) => {}
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        },

                        UiMode::Search => match key.code {
                            KeyCode::Esc => {
                                app.mode = UiMode::Normal;
                                app.dirty = true;
                            }
                            KeyCode::Enter => {
                                if app.search_query.is_empty() {
                                    app.mode = UiMode::Normal;
                                    app.dirty = true;
                                    continue;
                                }
                                if QUERYBASE.get().is_none() {
                                    continue;
                                }
                                let mut ress = search_result(&app.search_query).await;
                                while ress.is_err() {
                                    ress = search_result(&app.search_query).await;
                                }
                                let res = ress.unwrap();
                                app.search_results = res
                                    .get("data")
                                    .and_then(|v| v.get("items"))
                                    .and_then(Value::as_array)
                                    .unwrap()
                                    .iter()
                                    .take(10)
                                    .cloned()
                                    .collect();
                                if app.search_results.is_empty() {
                                    app.mode = UiMode::Normal;
                                    app.dirty = true;
                                    continue;
                                } else {
                                    app.selected = 0;
                                    app.mode = UiMode::Results;
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Backspace => {
                                app.dirty = true;
                                app.search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.dirty = true;
                            }
                            _ => {}
                        },

                        UiMode::Results => match key.code {
                            KeyCode::Esc => {
                                app.mode = UiMode::Normal;
                                app.dirty = true;
                            }
                            KeyCode::Up => {
                                if app.selected > 0 {
                                    app.selected -= 1;
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Down => {
                                if app.selected + 1 < app.search_results.len() {
                                    app.selected += 1;
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Enter => {
                                let index = (app.selected + 1).to_string();
                                add_song(
                                    &mut names,
                                    &mut urls,
                                    current,
                                    &app.search_results,
                                    index,
                                    app.search_query.clone(),
                                    tx.clone(),
                                )
                                .await;
                                app.queue_len = (urls.len() - current - 1) as i64;
                                app.dirty = true;
                                if mpv.get_property::<i64>("playlist-pos").unwrap() == -1 {
                                    if current != 0 {
                                        current += 1;
                                    }
                                    app.status =
                                        concat_strings(Vec::from(["Playing ", &names[current]]));
                                    if urls[current].starts_with("<?xml") {
                                        queue_mpd_song(&mut mpv, &urls[current]);
                                    } else {
                                        queue_song(&mut mpv, &urls[current]);
                                    }
                                }
                                app.mode = UiMode::Normal;
                            }
                            _ => {}
                        },
                    },
                    Event::Resize(_, _) => {
                        let area = terminal.size().unwrap();

                        if area.width != last.width || area.height != last.height {
                            last = area;

                            terminal
                                .draw(|f| {
                                    if current != names.len() {
                                        draw_ui(f, &app, &names[current + 1..]);
                                    } else {
                                        draw_ui(f, &app, &names[current..]);
                                    }
                                })
                                .unwrap();
                            app.dirty = false;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    restore_terminal(terminal);
}
