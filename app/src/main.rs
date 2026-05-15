use crossterm::event::{Event, KeyCode};
use libmpv2::{
    Format, Mpv,
    events::{Event as eve, PropertyData},
};
use macros::*;
use network::*;
use player::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio::time;
use ui::*;

fn print_help() {
    println!(
        "\nOxideAudio - Rust based music streamer.\n\n[Options] \n \t -c \t Clear cache files \n\t -r \t Turn off recommendations \n\t -n \t Switch to standard AAC format (data saver) \n\t -s \t Turn off song caching \n\t -u \t Update cache index \n\t -h \t Print this help message"
    );
}

fn advance_playback(mpv: &mut Mpv, urls: &[Value], current: &mut usize, app: &mut App) {
    if *current + 1 >= urls.len() {
        app.status = "Nothing is playing".to_string();
        app.queue_len = 0;
        app.dur = 0;
        app.dirty = true;
        *current = urls.len();
        if !mpv.get_property::<bool>("idle-active").unwrap() {
            mpv.command("seek", &["100", "absolute-percent"])
                .unwrap_or(());
        }
        return;
    }
    *current += 1;
    queue_song(mpv, &urls[*current]);
    app.queue_len = (urls.len() - *current - 1) as i64;
    let mut duration = urls[*current].get("duration").and_then(Value::as_i64);
    while duration.is_none() {
        let dur = mpv.get_property::<f64>("duration");
        if !dur.is_err() {
            duration = Some(dur.unwrap() as i64);
        }
    }
    app.dur = duration.unwrap();
    app.status = urls[*current]
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    app.dirty = true;
}
fn rewind_playback(mpv: &mut Mpv, urls: &[Value], current: &mut usize, app: &mut App) {
    if *current == 0 {
        match mpv.command("seek", &["0", "absolute-percent"]) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("{e}");
            }
        }
        app.dur = urls[*current]
            .get("duration")
            .and_then(Value::as_i64)
            .unwrap_or(0);
    } else if *current > 0 && urls.len() > 0 {
        queue_song(mpv, &urls[*current - 1]);
        *current -= 1;
        app.dur = match urls[*current].get("duration").and_then(Value::as_i64) {
            Some(e) => e,
            None => {
                let mut dur = mpv.get_property::<i64>("duration");
                while dur.is_err() {
                    dur = mpv.get_property::<i64>("duration");
                }
                dur.unwrap()
            }
        };
        app.status = urls[*current]
            .get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        app.queue_len = (urls.len() - *current - 1) as i64;
        app.dirty = true;
    } else if !mpv.get_property::<bool>("idle-active").unwrap() {
        match mpv.command("seek", &["0", "absolute"]) {
            Ok(_) => {
                let mut duration = urls[*current].get("duration").and_then(Value::as_i64);
                while duration.is_none() {
                    let dur = mpv.get_property::<f64>("duration");
                    if !dur.is_err() {
                        duration = Some(dur.unwrap() as i64);
                    }
                }
                app.dur = duration.unwrap();
                app.dur = duration.unwrap() as i64;
                app.dirty = true;
            }
            Err(_) => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let mut qual = "LOSSLESS";
    let mut update = false;
    let mut clear = false;
    let mut recs = true;
    let args: Vec<String> = env::args().collect();
    let mut save = false;
    let path = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/songs/",
        "i",
    ]))
    .rsplit_once("/")
    .unwrap()
    .0
    .to_string();
    if args.len() > 1 {
        for i in &args[1..] {
            if i == "-c" {
                clear = true;
            } else if i == "-r" {
                recs = false;
            } else if i == "-s" {
                save = true;
            } else if i == "-n" {
                qual = "HIGH";
            } else if i == "-u" {
                update = true;
            } else if i == "-h" {
                print_help();
                return;
            }
        }
    }
    if clear {
        match fs::remove_dir_all(&path) {
            Ok(_v) => {
                println!("Removed cache files");
            }
            Err(e) => {
                println!("Error removing direcotry:  {}, {}", path, e);
            }
        }
        return;
    }
    if !recs {
        RECS.store(false, Ordering::Relaxed);
    }
    if update {
        let mut jsn = global_json().lock().unwrap_or_else(|e| e.into_inner());
        if jsn.get("JSONversion").and_then(Value::as_str).is_none() {
            *jsn = convert_to_v1(&jsn);
            save_cache(&jsn);
            println!("cache file updated!");
            return;
        }
    }
    set_url();
    PREF_QUAL
        .set(qual.to_string())
        .expect("Quality already specified");
    SAVE_DATA.set(save).expect("Already set");
    let (tx, rx): (Sender<QueueItem>, Receiver<QueueItem>) = mpsc::channel();
    let (cache_send, cache_recv): (Sender<CacheItem>, Receiver<CacheItem>) = mpsc::channel();

    let mut mpv = match Mpv::with_initializer(|_init| Ok(())) {
        Ok(player) => player,
        Err(e) => {
            println!("Failed to start MPV: {}", e);
            return;
        }
    };
    let mut mpv2 = Mpv::new().unwrap();
    mpv.set_property(
        "demuxer-lavf-o",
        "protocol_whitelist=[file,https,http,tls,tcp,crypto,data]",
    )
    .unwrap();
    mpv2.set_property(
        "demuxer-lavf-o",
        "protocol_whitelist=[file,https,http,tls,tcp,crypto,data]",
    )
    .unwrap();
    // mpv.set_property("msg-level", "all=debug").unwrap();
    // mpv.set_property("log-file", "./mpv.log").unwrap();
    // mpv2.set_property("log-file", "./mpv2.log").unwrap();

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
        track_format: String::new(),
        sample_rate: 0,
        channel_count: 0,
        bitrate: 0,
        msg: String::new(),
    };
    mpv.observe_property("time-pos", Format::Double, 3).unwrap();
    mpv2.observe_property("time-pos", Format::Double, 3)
        .unwrap();
    mpv.observe_property("track-list", Format::String, 4)
        .unwrap();
    mpv2.observe_property("track-list", Format::String, 4)
        .unwrap();
    let options = Options { queue: true };
    let mut last_mode_switch = Instant::now() - Duration::from_secs(1);
    let mut last_add;
    let mut last_update = Instant::now();
    let mut last_msg = Instant::now();
    let update_every = Duration::from_millis(100);
    let skip_every = Duration::from_millis(800);
    let mut last = terminal.size().unwrap();
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
            app.status = urls[current]
                .get("name")
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
                crossfade(&mut mpv, &mut mpv2, &urls[current]);
                player_num = 2;
                match urls[current].get("duration").and_then(Value::as_i64) {
                    Some(e) => dura = e as f64,
                    None => {}
                }
                match mpv2.get_property::<f64>("time-pos") {
                    Ok(e) => tim = e,
                    Err(_) => {}
                }
                app.dur = dura.round() as i64;
                app.cur_time = tim.round() as i64;
            } else if player_num == 2 {
                crossfade(&mut mpv2, &mut mpv, &urls[current]);
                player_num = 1;
                match urls[current].get("duration").and_then(Value::as_i64) {
                    Some(e) => dura = e as f64,
                    None => {}
                }
                match mpv.get_property::<f64>("time-pos") {
                    Ok(e) => tim = e,
                    Err(_) => {}
                }
                app.cur_time = tim.round() as i64;
                app.dur = dura.round() as i64;
            }
            while crossterm::event::poll(Duration::from_millis(0)).unwrap() {
                let _ = crossterm::event::read();
            }
        }

        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        if RECS.load(Ordering::Relaxed)
            && !IS_RUNNING.load(Ordering::SeqCst)
            && current + 1 == urls.len()
        {
            if urls.len() > 1 {
                spawn_recommendation_worker(urls.last().unwrap().to_string(), tx.clone());
            }
        }
        if urls.len() > current + 1 {
            if !urls[current + 1]
                .get("url")
                .and_then(Value::as_str)
                .unwrap()
                .starts_with("/")
                && if player_num == 1 {
                    mpv.get_property::<i64>("percent-pos").unwrap_or(0) > 30
                } else {
                    mpv2.get_property::<i64>("percent-pos").unwrap_or(0) > 30
                }
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
        if !app.msg.is_empty() && (Instant::now() - last_msg) > Duration::from_secs(3) {
            app.msg = String::new();
            app.dirty = true;
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
                        eve::PropertyChange {
                            name: _,
                            change: PropertyData::Str(e),
                            reply_userdata: 4,
                        } => {
                            let newvec = serde_json::from_str::<Vec<Value>>(e).unwrap();
                            if newvec.is_empty() {
                                continue;
                            }
                            let json = newvec.get(0).unwrap();
                            app.channel_count =
                                json.get("audio-channels").and_then(Value::as_i64).unwrap();
                            app.sample_rate = json
                                .get("demux-samplerate")
                                .and_then(Value::as_i64)
                                .unwrap();
                            app.track_format = json
                                .get("codec")
                                .and_then(Value::as_str)
                                .unwrap()
                                .to_uppercase();
                            let bitrate = json.get("demux-bitrate").and_then(Value::as_i64);
                            if !bitrate.is_none() {
                                app.bitrate = bitrate.unwrap()
                            } else {
                                let bitrate = mpv.get_property::<i64>("audio-bitrate");
                                if !bitrate.is_err() {
                                    app.bitrate = bitrate.unwrap();
                                } else {
                                    let format = json.get("format-name").and_then(Value::as_str);
                                    if format.is_none() {
                                        app.bitrate = 0;
                                        continue;
                                    }
                                    if qual == "HIGH" {
                                        app.bitrate = 320;
                                    } else if qual == "LOW" {
                                        app.bitrate = 96;
                                    } else {
                                        let form = format.unwrap();
                                        app.bitrate = app.sample_rate
                                            * app.channel_count
                                            * if form == "s16" { 16 } else { 24 }
                                    }
                                }
                            }
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
                        eve::PropertyChange {
                            name: _,
                            change: PropertyData::Str(e),
                            reply_userdata: 4,
                        } => {
                            let newvec = serde_json::from_str::<Vec<Value>>(e).unwrap();
                            if newvec.is_empty() {
                                continue;
                            }
                            let json = newvec.get(0).unwrap();
                            app.channel_count =
                                json.get("audio-channels").and_then(Value::as_i64).unwrap();
                            app.sample_rate = json
                                .get("demux-samplerate")
                                .and_then(Value::as_i64)
                                .unwrap();
                            app.track_format = json
                                .get("codec")
                                .and_then(Value::as_str)
                                .unwrap()
                                .to_uppercase();
                            let bitrate = json.get("demux-bitrate").and_then(Value::as_i64);
                            if !bitrate.is_none() {
                                app.bitrate = bitrate.unwrap()
                            } else {
                                let bitrate = mpv2.get_property::<i64>("audio-bitrate");
                                if !bitrate.is_err() {
                                    app.bitrate = bitrate.unwrap();
                                } else {
                                    let format = json.get("format-name").and_then(Value::as_str);
                                    if format.is_none() {
                                        app.bitrate = 0;
                                        continue;
                                    }
                                    if qual == "HIGH" {
                                        app.bitrate = 320;
                                    } else if qual == "LOW" {
                                        app.bitrate = 96;
                                    } else {
                                        let form = format.unwrap();
                                        app.bitrate = app.sample_rate
                                            * app.channel_count
                                            * if form == "s16" { 16 } else { 24 }
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        if mpv.get_property::<bool>("idle-active").unwrap()
            && mpv2.get_property::<bool>("idle-active").unwrap()
        {
            if last_mode_switch.elapsed() >= skip_every + Duration::from_secs(3)
                && urls.len() > current + 1
            {
                app.cur_time = 0;
                app.dur = 0;
                // current -= 1;
                if player_num == 1 {
                    advance_playback(&mut mpv, &urls, &mut current, &mut app);
                } else if player_num == 2 {
                    advance_playback(&mut mpv2, &urls, &mut current, &mut app);
                }
                dura = app.dur as f64;
                last_mode_switch = Instant::now();
                app.dirty = true;
            } else {
                app.dur = 0;
                app.cur_time = 0;
                app.status = "Nothing is playing".to_string();
                current = urls.len();
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
                            KeyCode::Char('h') | KeyCode::Char('H') => {
                                app.dirty = true;
                                SHUTDOWN.store(true, Ordering::SeqCst);
                                break;
                            }
                            KeyCode::Char('p') | KeyCode::Char(' ') | KeyCode::Char('P') => {
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
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                app.cur_time = 0;
                                tim = 0.0;
                                app.dur = 0;
                                if player_num == 1 {
                                    rewind_playback(&mut mpv, &urls, &mut current, &mut app);
                                } else if player_num == 2 {
                                    rewind_playback(&mut mpv2, &urls, &mut current, &mut app);
                                }
                                dura = app.dur as f64;
                                app.dirty = true;
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                app.dur = 0;
                                app.cur_time = 0;
                                tim = 0.0;
                                if player_num == 1 {
                                    advance_playback(&mut mpv, &urls, &mut current, &mut app);
                                } else if player_num == 2 {
                                    advance_playback(&mut mpv2, &urls, &mut current, &mut app);
                                }
                                app.dirty = true;
                                dura = app.dur as f64;
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                app.dirty = true;
                                app.search_query.clear();
                                app.mode = UiMode::Search;
                            }
                            KeyCode::Char('f') | KeyCode::Char('F') => {
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
                                    if ((app.dur - app.cur_time) as f64) < CROSSFADE_DUR {
                                        app.cur_time = app.dur;
                                        break;
                                    }
                                    app.dirty = true;
                                }
                            }
                            KeyCode::Char('b') | KeyCode::Char('B') => {
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
                                app.cur_time = std::cmp::max(app.cur_time - 5, 0);
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
                                let mut fallback_used = false;
                                let mut ress = search_result(&app.search_query).await;
                                if ress.is_err() {
                                    fallback_used = true;
                                    ress = fallback_search(&app.search_query).await;
                                }
                                if ress.is_err() {
                                    app.msg = "Couldn't complete the search".to_string();
                                    last_msg = Instant::now();
                                    app.mode = UiMode::Normal;
                                    app.dirty = true;
                                    continue;
                                }
                                let res = ress.unwrap();
                                let mut new_jsn = Vec::new();
                                if fallback_used {
                                    if let Some(v) = res.get("items").and_then(Value::as_array) {
                                        for i in v {
                                            let name = i
                                                .get("performer")
                                                .and_then(|v| v.get("name"))
                                                .and_then(Value::as_str)
                                                .unwrap_or("Unknown");
                                            let duration = i
                                                .get("duration")
                                                .and_then(Value::as_i64)
                                                .unwrap_or(0);
                                            let id = i
                                                .get("id")
                                                .and_then(Value::as_i64)
                                                .unwrap_or(0)
                                                .to_string();
                                            let title = i
                                                .get("title")
                                                .and_then(Value::as_str)
                                                .unwrap_or("Unknown");
                                            new_jsn.push(json!({"artist": name, "duration": duration, "id": id, "title": title, "source": "qobuz"}));
                                        }
                                    }
                                } else {
                                    if let Some(v) = res
                                        .get("results")
                                        .and_then(Value::as_array)
                                        .and_then(|v| v.first())
                                        .and_then(|v| v.get("hits"))
                                        .and_then(Value::as_array)
                                    {
                                        for i in v {
                                            let name = i
                                                .get("document")
                                                .and_then(|v| v.get("artistName"))
                                                .and_then(Value::as_str)
                                                .unwrap_or("Unknown");
                                            let duration = i
                                                .get("document")
                                                .and_then(|v| v.get("duration"))
                                                .and_then(Value::as_i64)
                                                .unwrap_or(0);
                                            let id = i
                                                .get("document")
                                                .and_then(|v| v.get("asin"))
                                                .and_then(Value::as_str)
                                                .unwrap_or("");
                                            let title = i
                                                .get("document")
                                                .and_then(|v| v.get("title"))
                                                .and_then(Value::as_str)
                                                .unwrap_or("Unknown");
                                            new_jsn.push(json!({"artist": name, "duration": duration, "id": id, "title": title, "source": "amazon"}));
                                        }
                                    }
                                }
                                app.search_results = new_jsn;
                                if app.search_results.is_empty() {
                                    let results = get_suggestions(&app.search_query).await;
                                    if results.is_err() {
                                        app.msg = "No suggestions, try different query".to_string();
                                        last_msg = Instant::now();
                                        app.mode = UiMode::Normal;
                                        app.dirty = true;
                                        continue;
                                    } else {
                                        let res = results.unwrap();
                                        app.mode = UiMode::Suggestions;
                                        app.search_results.clear();
                                        if let Some(a) = res.get("tracks").and_then(Value::as_array)
                                        {
                                            for i in a {
                                                let name = i
                                                    .get("artist")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("Unknown");
                                                let title = i
                                                    .get("title")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("Unknown");
                                                let id = i
                                                    .get("id")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("");
                                                let duration = i
                                                    .get("duration")
                                                    .and_then(Value::as_i64)
                                                    .unwrap_or(0);
                                                app.search_results.push(json!({"artist": name, "duration": duration, "id": id, "title": title, "source": "qobuz"}));
                                            }
                                        }
                                    }
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
                                let index = (app.selected).to_string();
                                if !add_song(
                                    &mut urls,
                                    current,
                                    &app.search_results,
                                    index,
                                    app.search_query.clone(),
                                    tx.clone(),
                                )
                                .await
                                {
                                    app.msg = String::from("Couldn't queue the last request");
                                    last_msg = Instant::now();
                                    app.mode = UiMode::Normal;
                                    app.dirty = true;
                                    continue;
                                }
                                app.queue_len = (urls.len() - current - 1) as i64;
                                app.dirty = true;
                                if mpv.get_property::<i64>("playlist-pos").unwrap() == -1
                                    && player_num == 1
                                {
                                    if current != 0 {
                                        current += 1;
                                    }
                                    app.status = urls[current]
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .unwrap()
                                        .to_string();
                                    app.dur =
                                        match urls[current].get("duration").and_then(Value::as_i64)
                                        {
                                            Some(e) => {
                                                dura = e as f64;
                                                e
                                            }
                                            None => 0,
                                        };
                                    queue_song(&mut mpv, &urls[current]);
                                } else if mpv2.get_property::<i64>("playlist-pos").unwrap() == -1
                                    && player_num == 2
                                {
                                    if current != 0 {
                                        current += 1;
                                    }
                                    app.status = urls[current]
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .unwrap()
                                        .to_string();
                                    app.dur =
                                        match urls[current].get("duration").and_then(Value::as_i64)
                                        {
                                            Some(e) => {
                                                dura = e as f64;
                                                e
                                            }
                                            None => 0,
                                        };
                                    queue_song(&mut mpv2, &urls[current]);
                                }
                                app.mode = UiMode::Normal;
                            }
                            _ => {}
                        },
                        UiMode::Suggestions => match key.code {
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
                                if app.selected < app.search_results.len() {
                                    let title = app.search_results[app.selected]
                                        .get("title")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Unknown");
                                    let artist = app.search_results[app.selected]
                                        .get("artist")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Unknown");
                                    let query = title.to_string() + " " + artist;
                                    let mut fallback_used = false;
                                    let mut ress = search_result(&query).await;
                                    if ress.is_err() {
                                        fallback_used = true;
                                        ress = fallback_search(&query).await;
                                    }
                                    if ress.is_err() {
                                        app.msg = "No results from suggested input.".to_string();
                                        last_msg = Instant::now();
                                        app.dirty = true;
                                        continue;
                                    } else {
                                        let res = ress.unwrap();
                                        let mut new_jsn = Vec::new();
                                        if fallback_used {
                                            if let Some(v) =
                                                res.get("items").and_then(Value::as_array)
                                            {
                                                for i in v {
                                                    let name = i
                                                        .get("performer")
                                                        .and_then(|v| v.get("name"))
                                                        .and_then(Value::as_str)
                                                        .unwrap_or("Unknown");
                                                    let duration = i
                                                        .get("duration")
                                                        .and_then(Value::as_i64)
                                                        .unwrap_or(0);
                                                    let id = i
                                                        .get("id")
                                                        .and_then(Value::as_i64)
                                                        .unwrap_or(0)
                                                        .to_string();
                                                    let title = i
                                                        .get("title")
                                                        .and_then(Value::as_str)
                                                        .unwrap_or("Unknown");
                                                    new_jsn.push(json!({"artist": name, "duration": duration, "id": id, "title": title, "source": "qobuz"}));
                                                }
                                            }
                                        } else {
                                            if let Some(v) = res
                                                .get("results")
                                                .and_then(Value::as_array)
                                                .and_then(|v| v.first())
                                                .and_then(|v| v.get("hits"))
                                                .and_then(Value::as_array)
                                            {
                                                for i in v {
                                                    let name = i
                                                        .get("document")
                                                        .and_then(|v| v.get("artistName"))
                                                        .and_then(Value::as_str)
                                                        .unwrap_or("Unknown");
                                                    let duration = i
                                                        .get("document")
                                                        .and_then(|v| v.get("duration"))
                                                        .and_then(Value::as_i64)
                                                        .unwrap_or(0);
                                                    let id = i
                                                        .get("document")
                                                        .and_then(|v| v.get("asin"))
                                                        .and_then(Value::as_str)
                                                        .unwrap_or("");
                                                    let title = i
                                                        .get("document")
                                                        .and_then(|v| v.get("title"))
                                                        .and_then(Value::as_str)
                                                        .unwrap_or("Unknown");
                                                    new_jsn.push(json!({"artist": name, "duration": duration, "id": id, "title": title, "source": "amazon"}));
                                                }
                                            }
                                        }
                                        app.search_results = new_jsn;
                                        if app.search_results.is_empty() {
                                            app.msg = "No results".to_string();
                                            app.mode = UiMode::Normal;
                                            last_msg = Instant::now();
                                            app.dirty = true;
                                            continue;
                                        } else {
                                            app.selected = 0;
                                            app.mode = UiMode::Results;
                                            app.dirty = true;
                                        }
                                    }
                                }
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
