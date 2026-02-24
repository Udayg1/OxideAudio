use crossterm::event::{Event, KeyCode};
use libmpv2::{
    Format, Mpv,
    events::{Event as eve, PropertyData},
};
use macros::*;
use network::*;
use player::*;
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio::time;
use ui::*;

fn advance_playback(mpv: &mut Mpv, urls: &[Value], current: &mut usize, app: &mut App) {
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
    app.status = concat_strings(Vec::from([
        "Playing ",
        urls[*current].get("name").and_then(Value::as_str).unwrap(),
    ]));
    app.image = urls[*current]
        .get("image")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    if urls[*current]
        .get("url")
        .and_then(Value::as_str)
        .unwrap()
        .starts_with("<?xml")
    {
        queue_mpd_song(
            mpv,
            urls[*current].get("url").and_then(Value::as_str).unwrap(),
        );
    } else {
        queue_song(
            mpv,
            urls[*current].get("url").and_then(Value::as_str).unwrap(),
        );
    }

    app.dirty = true;
}
fn rewind_playback(mpv: &mut Mpv, urls: &[Value], current: &mut usize, app: &mut App) {
    if *current > 0 && urls.len() > 0 {
        if urls[*current - 1]
            .get("url")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("<?xml")
        {
            queue_mpd_song(
                mpv,
                urls[*current - 1]
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap(),
            );
        } else {
            queue_song(
                mpv,
                urls[*current - 1]
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap(),
            );
        }
        *current -= 1;
        app.status = concat_strings(Vec::from([
            "Playing ",
            &urls[*current].get("name").and_then(Value::as_str).unwrap(),
        ]));
        app.image = urls[*current]
            .get("image")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        app.queue_len = (urls.len() - *current - 1) as i64;
        app.dirty = true;
    } else if !mpv.get_property::<bool>("idle-active").unwrap() {
        match mpv.command("seek", &["0", "absolute"]) {
            Ok(_) => {}
            Err(_) => {}
        }
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
    let (cache_send, cache_recv): (Sender<CacheItem>, Receiver<CacheItem>) = mpsc::channel();
    let log_file_location = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/mpv.log",
    ]));
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
    let mut mpv2 = Mpv::new().unwrap();
    mpv.set_property(
        "demuxer-lavf-o",
        "protocol_whitelist=[file,https,http,tls,tcp,crypto,data]",
    )
    .unwrap();
    // mpv.set_property("msg-level", "all=debug").unwrap();
    // mpv.set_property("log-file", log_file_location.to_string()).unwrap();
    let mut terminal = setup_terminal();
    let mut urls: Vec<Value> = Vec::new();
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
        cur_time: 0,
        dur: 0,
        image: String::new(),
    };
    mpv.observe_property("idle-active", Format::Flag, 2)
        .unwrap();
    mpv.observe_property("time-pos", Format::Double, 3).unwrap();
    mpv2.observe_property("time-pos", Format::Double, 3)
        .unwrap();
    mpv.observe_property("duration", Format::Double, 1).unwrap();
    mpv2.observe_property("duration", Format::Double, 1)
        .unwrap();
    let mut options = Options { queue: true };
    let mut last_mode_switch = Instant::now() - Duration::from_secs(1);
    let mut last_add;
    let mut last_update = Instant::now();
    let update_every = Duration::from_millis(100);
    let skip_every = Duration::from_millis(800);
    let mut _auto_started = false;
    let mut _skipped = false;
    let mut last = terminal.size().unwrap();
    let mut _logfile = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(log_file_location)
        .unwrap();
    let mut player_num = 1;
    let mut dura = 0.0;
    let mut tim = 0.0;
    loop {
        last_add = 0;
        if (!mpv.get_property::<bool>("idle-active").unwrap()
            || !mpv2.get_property::<bool>("idle-active").unwrap())
            && dura != 0.0
            && dura - tim < CROSSFADE_DUR
            && current + 1 < urls.len()
        {
            app.cur_time = 0;
            app.dur = 0;
            current += 1;
            app.status = concat_strings(Vec::from([
                "Playing ",
                urls[current].get("name").and_then(Value::as_str).unwrap(),
            ]));
            app.image = urls[current]
                .get("image")
                .and_then(Value::as_str)
                .unwrap()
                .to_string();
            app.queue_len = (urls.len() - current - 1) as i64;
            if current != urls.len() {
                terminal
                    .draw(|f| draw_ui(f, &mut app, &urls[current + 1..], &options))
                    .unwrap();
            } else {
                terminal
                    .draw(|f| draw_ui(f, &mut app, &urls[current..], &options))
                    .unwrap();
            }
            if player_num == 1 {
                crossfade(
                    &mut mpv,
                    &mut mpv2,
                    urls[current]
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap()
                        .to_string(),
                );
                player_num = 2;
                dura = mpv2.get_property("duration").unwrap();
                app.dur = dura.round() as i64;
                tim = mpv2.get_property("time-pos").unwrap();
                app.cur_time = tim.round() as i64;
            } else if player_num == 2 {
                crossfade(
                    &mut mpv2,
                    &mut mpv,
                    urls[current]
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap()
                        .to_string(),
                );
                player_num = 1;
                dura = mpv.get_property("duration").unwrap();
                tim = mpv.get_property("time-pos").unwrap();
                app.cur_time = tim.round() as i64;
                app.dur = dura.round() as i64;
            }
            while crossterm::event::poll(Duration::from_millis(0)).unwrap() && last_add < 5 {
                last_add += 1;
                let _ = crossterm::event::read();
            }
        }
        last_add = 0;

        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        if !IS_RUNNING.load(Ordering::SeqCst) && current + 1 == urls.len() {
            if urls.len() > 1 {
                spawn_recommendation_worker(urls.last().unwrap().to_string(), tx.clone());
            }
        }
        if urls.len() > current + 1 {
            if !check_song(urls[current + 1].get("id").and_then(Value::as_str).unwrap())
                && !urls[current + 1]
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap()
                    .starts_with("/")
            {
                if save {
                    if !IS_CACHING.load(Ordering::SeqCst) {
                        if let Ok(data) = cache_recv.try_recv() {
                            urls[data.index]["url"] = json!(data.path);
                        } else {
                            cache_next_song(
                                urls[current + 1]
                                    .get("url")
                                    .and_then(Value::as_str)
                                    .unwrap()
                                    .to_string(),
                                current + 1,
                                cache_send.clone(),
                            );
                        }
                    }
                }
            }
        }
        if player_num == 1 {
            if let Some(event) = mpv.wait_event(0.05) {
                match event {
                    Ok(e) => match e {
                        eve::PropertyChange {
                            change: PropertyData::Double(f),
                            reply_userdata: 1,
                            ..
                        } => {
                            dura = f;
                            app.dur = f.round() as i64;
                        }
                        eve::PropertyChange {
                            change: PropertyData::Double(f),
                            reply_userdata: 3,
                            ..
                        } => {
                            tim = f;
                            app.cur_time = f.round() as i64;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        } else if player_num == 2 {
            if let Some(event) = mpv2.wait_event(0.05) {
                match event {
                    Ok(e) => match e {
                        eve::PropertyChange {
                            change: PropertyData::Double(f),
                            reply_userdata: 1,
                            ..
                        } => {
                            dura = f;
                            app.dur = f.round() as i64;
                        }
                        eve::PropertyChange {
                            change: PropertyData::Double(f),
                            reply_userdata: 3,
                            ..
                        } => {
                            tim = f;
                            app.cur_time = f.round() as i64;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        if mpv.get_property::<bool>("idle-active").unwrap()
            && mpv2.get_property::<bool>("idle-active").unwrap()
            && urls.len() >= current + 1
        {
            if last_mode_switch.elapsed() >= skip_every {
                app.cur_time = 0;
                app.dur = 0;
                if player_num == 1 {
                    advance_playback(&mut mpv, &urls, &mut current, &mut app);
                } else if player_num == 2 {
                    advance_playback(&mut mpv2, &urls, &mut current, &mut app);
                }
                last_mode_switch = Instant::now();
                _auto_started = true;
            }
        }

        if app.dirty || Instant::now() - last_update > update_every {
            if current != urls.len() {
                terminal
                    .draw(|f| draw_ui(f, &mut app, &urls[current + 1..], &options))
                    .unwrap();
            } else {
                terminal
                    .draw(|f| draw_ui(f, &mut app, &urls[current..], &options))
                    .unwrap();
            }
            app.dirty = false;
            last_update = Instant::now();
        }
        while let Ok(item) = rx.try_recv() {
            match item {
                QueueItem::Url(url) => {
                    urls.push(url);
                    last_add += 1;
                    app.queue_len += 1;
                }
            }
            app.dirty = true;
            if last_add > 5 {
                break;
            }
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
                            KeyCode::Char('u') => {
                                app.dirty = true;
                                options.queue = !options.queue;
                            }
                            KeyCode::Char('p') => {
                                if !app.paused {
                                    if player_num == 1 {
                                        mpv.set_property("pause", true).unwrap();
                                    } else {
                                        mpv2.set_property("pause", true).unwrap();
                                    }
                                    app.paused = true;
                                    app.dirty = true;
                                } else if app.paused {
                                    if player_num == 1 {
                                        mpv.set_property("pause", false).unwrap();
                                    } else {
                                        mpv2.set_property("pause", false).unwrap();
                                    }
                                    app.paused = false;
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Char('r') => {
                                _skipped = true;
                                app.cur_time = 0;
                                app.dur = 0;
                                if player_num == 1 {
                                    rewind_playback(&mut mpv, &urls, &mut current, &mut app);
                                } else if player_num == 2 {
                                    rewind_playback(&mut mpv2, &urls, &mut current, &mut app);
                                }
                                app.dirty = true;
                            }
                            KeyCode::Char('s') => {
                                _skipped = true;
                                app.dur = 0;
                                app.cur_time = 0;
                                if player_num == 1 {
                                    advance_playback(&mut mpv, &urls, &mut current, &mut app);
                                } else if player_num == 2 {
                                    advance_playback(&mut mpv2, &urls, &mut current, &mut app);
                                }
                                app.dirty = true;
                            }
                            KeyCode::Char('a') => {
                                app.dirty = true;
                                app.search_query.clear();
                                app.mode = UiMode::Search;
                            }
                            KeyCode::Char('f') => {
                                if !mpv.get_property::<bool>("idle-active").unwrap()
                                    || !mpv2.get_property::<bool>("idle-active").unwrap()
                                {
                                    if player_num == 1 {
                                        match mpv.command("seek", &["5", "relative"]) {
                                            Ok(()) => {}
                                            _ => {}
                                        }
                                    } else if player_num == 2 {
                                        match mpv2.command("seek", &["5", "relative"]) {
                                            Ok(()) => {}
                                            _ => {}
                                        }
                                    }
                                    app.cur_time += 5;
                                    if app.cur_time > app.dur {
                                        app.cur_time = app.dur;
                                    }
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Char('b') => {
                                if !mpv.get_property::<bool>("idle-active").unwrap()
                                    || !mpv2.get_property::<bool>("idle-active").unwrap()
                                {
                                    if player_num == 1 {
                                        match mpv.command("seek", &["-5", "relative"]) {
                                            Ok(()) => {}
                                            _ => {}
                                        }
                                    } else if player_num == 2 {
                                        match mpv2.command("seek", &["-5", "relative"]) {
                                            Ok(()) => {}
                                            _ => {}
                                        }
                                    }
                                }
                                app.cur_time -= 5;
                                if app.cur_time > app.dur {
                                    app.cur_time = app.dur;
                                }
                                app.dirty = true;
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
                                    app.status = concat_strings(Vec::from([
                                        "Playing ",
                                        &urls[current].get("name").and_then(Value::as_str).unwrap(),
                                    ]));
                                    app.image = urls[current]
                                        .get("image")
                                        .and_then(Value::as_str)
                                        .unwrap()
                                        .to_string();
                                    eprintln!("{}", app.image);
                                    if urls[current]
                                        .get("url")
                                        .and_then(Value::as_str)
                                        .unwrap()
                                        .starts_with("<?xml")
                                    {
                                        queue_mpd_song(
                                            &mut mpv,
                                            &urls[current]
                                                .get("url")
                                                .and_then(Value::as_str)
                                                .unwrap(),
                                        );
                                    } else {
                                        queue_song(
                                            &mut mpv,
                                            &urls[current]
                                                .get("url")
                                                .and_then(Value::as_str)
                                                .unwrap(),
                                        );
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
                                    if current != urls.len() {
                                        draw_ui(f, &mut app, &urls[current + 1..], &options);
                                    } else {
                                        draw_ui(f, &mut app, &urls[current..], &options);
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
    for i in urls {
        let url = i.get("url").and_then(Value::as_str);
        if !url.is_none() {
            let furl = url.unwrap();
            if furl.starts_with(&env::temp_dir().display().to_string()) {
                match fs::remove_file(furl) {
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
        }
    }
}
