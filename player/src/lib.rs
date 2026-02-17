use base64::{Engine as _, engine::general_purpose};
use libmpv2::Mpv;
use macros::*;
use network::*;
use serde_json::Value;
use std::io::{Write, stderr};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio;
use uuid;

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
pub static ID_CACHE: OnceLock<Mutex<Value>> = OnceLock::new();
pub static SAVE_DATA: OnceLock<bool> = OnceLock::new();
pub static IS_RUNNING: AtomicBool = AtomicBool::new(false);
pub static CROSSFADE_DUR: f64 = 4.7;

pub enum QueueItem {
    Url(Vec<String>),
    Mpd(Vec<String>),
}

pub fn crossfade(mpv1: &mut Mpv, mpv2: &mut Mpv, new_song: String) {
    let cur_vol: f64 = mpv1.get_property("volume").unwrap();
    mpv2.set_property("volume", 0.0).unwrap();

    // Queue next track
    if new_song.starts_with("<xml") {
        queue_mpd_song(mpv2, &new_song);
    } else {
        queue_song(mpv2, &new_song);
    }

    let dur: f64 = mpv1.get_property("duration").unwrap();
    let start_time = Instant::now();

    while start_time.elapsed().as_secs_f64() < CROSSFADE_DUR {
        let prog = mpv1.get_property::<f64>("time-pos");
        if prog.is_err(){
            break;
        }
        
        let remaining = dur - prog.unwrap();

        let progress = if remaining >= CROSSFADE_DUR {
            0.0
        } else {
            1.0 - (remaining / CROSSFADE_DUR)
        };

        let vol1 = (1.0 - progress) * cur_vol;
        let vol2 = progress * cur_vol; // or 100.0 if you want full next track

        mpv1.set_property("volume", vol1).unwrap();
        mpv2.set_property("volume", vol2).unwrap();

        std::thread::sleep(Duration::from_millis(10));
    }

    // Ensure volumes are correct at the end
    mpv1.set_property("volume", 0.0).unwrap();
    mpv2.set_property("volume", cur_vol).unwrap();
}
pub fn check_song(id: &str) -> bool {
    let path = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/songs/",
        id,
    ]));
    fs::create_dir_all(&path.rsplit_once("/").unwrap().0).unwrap();
    let f = fs::File::open(path);
    if f.is_err() { false } else { true }
}

pub fn global_json() -> &'static Mutex<Value> {
    ID_CACHE.get_or_init(|| {
        let path = concat_strings(Vec::from([
            &env::var("HOME").unwrap(),
            "/.local/share/mscply/cache.json",
        ]));

        let value = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| empty_json());

        Mutex::new(value)
    })
}

pub fn save_cache(json: &std::sync::MutexGuard<'_, Value>) {
    let path = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/cache.json",
    ]));
    fs::create_dir_all(path.rsplit_once('/').unwrap().0).unwrap();
    fs::write(path, serde_json::to_string_pretty(&**json).unwrap()).unwrap();
}

pub fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut json = global_json().lock().unwrap();
            IS_RUNNING.store(true, Ordering::SeqCst);
            let new_iid = convert_to_ytm(&name).await.unwrap();
            let njson = get_ytrecs(&new_iid).await;
            let arr = get_ytrec_array(njson);
            stderr().flush().unwrap();
            let mut count = 0;
            for item in arr.iter() {
                if count > 10 {
                    save_cache(&json);
                    count = 0;
                } else {
                    count += 1;
                }
                if SHUTDOWN.load(Ordering::SeqCst) {
                    save_cache(&json);
                    return;
                }
                let name = item.get("name").and_then(Value::as_str).unwrap();
                let artist = item
                    .get("artist")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown");
                let id = item.get("id").and_then(Value::as_str).unwrap();
                let tidal_id = json.get(id).and_then(Value::as_str);
                let tidal_id_final: String;
                if tidal_id.is_none() {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);
                        return;
                    }
                    let songlink_data = get_songlink_data(id, "y").await;
                    let iiiid = extract_tidal_id(&songlink_data);
                    if iiiid.is_none() {
                        continue;
                    } else {
                        tidal_id_final = iiiid.unwrap();
                        json[id] = Value::String(tidal_id_final.clone());
                    }
                } else {
                    tidal_id_final = tidal_id.unwrap().to_string();
                }
                if !tidal_id_final.is_empty() {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);
                        return;
                    }
                    let cached = check_song(&tidal_id_final);
                    if cached {
                        tx.send(QueueItem::Url(Vec::from([
                            concat_strings(Vec::from([
                                &env::var("HOME").unwrap(),
                                "/.local/share/mscply/songs/",
                                &tidal_id_final,
                            ])),
                            concat_strings(Vec::from([name, " - ", artist])),
                        ])))
                        .ok();
                        continue;
                    }
                    let quality = get_quality(&tidal_id_final).await;
                    if quality.is_empty() {
                        continue;
                    }
                    let id: i32 = tidal_id_final.parse().unwrap();
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);
                        return;
                    }
                    if let Ok(res) = get_song(id, &quality).await {
                        let manifest = res
                            .get("data")
                            .and_then(|d| d.get("manifest"))
                            .and_then(Value::as_str);
                        if !manifest.is_none() {
                            let decoded = decode_base64(manifest.unwrap());
                            if decoded.starts_with("<?xml") {
                                if *SAVE_DATA.get().unwrap_or(&true) {
                                    tx.send(QueueItem::Url(Vec::from([
                                        decoded.to_string(),
                                        concat_strings(Vec::from([name, " - ", artist])),
                                    ])))
                                    .ok();
                                    continue;
                                }
                                cache_mpd_song(&decoded, &tidal_id_final).await;
                                tx.send(QueueItem::Mpd(Vec::from([
                                    concat_strings(Vec::from([
                                        &env::var("HOME").unwrap(),
                                        "/.local/share/mscply/songs/",
                                        &tidal_id_final,
                                    ])),
                                    concat_strings(Vec::from([name, " - ", artist])),
                                ])))
                                .ok();
                            } else if let Ok(json) = serde_json::from_str::<Value>(&decoded) {
                                if let Some(url) = json
                                    .get("urls")
                                    .and_then(|v| v.as_array())
                                    .and_then(|a| a.first())
                                    .and_then(Value::as_str)
                                {
                                    if *SAVE_DATA.get().unwrap_or(&true) {
                                        tx.send(QueueItem::Url(Vec::from([
                                            url.to_string(),
                                            concat_strings(Vec::from([name, " - ", artist])),
                                        ])))
                                        .ok();
                                        continue;
                                    }
                                    cache_url(&tidal_id_final, url).await;
                                    tx.send(QueueItem::Url(Vec::from([
                                        concat_strings(Vec::from([
                                            &env::var("HOME").unwrap(),
                                            "/.local/share/mscply/songs/",
                                            &tidal_id_final,
                                        ])),
                                        concat_strings(Vec::from([name, " - ", artist])),
                                    ])))
                                    .ok();
                                }
                            }
                        }
                    }
                }
            }
            save_cache(&json);
            IS_RUNNING.store(false, Ordering::SeqCst);
        });
    });
}

pub fn queue_mpd_song(mpv: &mut Mpv, mpd: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    let path = concat_strings(Vec::from([
        &env::temp_dir().display().to_string(),
        "/mpd_",
        &uuid::Uuid::new_v4().to_string(),
    ]));
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file_write(&mut f, mpd).unwrap();
    f.flush().unwrap();
    queue_song(mpv, &path);
}

pub fn queue_song(mpv: &mut Mpv, url: &str) {
    mpv.command("loadfile", &[url, "replace"]).unwrap();
}

fn decode_base64(encoded: &str) -> String {
    let stripped = encoded.trim();
    let mut t = stripped.replace("-", "+").replace("_", "/");
    let missing = t.len() % 4;
    if missing == 1 {
        return String::from(stripped);
    } else if missing == 2 {
        t.push_str("==");
    } else if missing == 3 {
        t.push_str("=");
    }
    let decoded = general_purpose::STANDARD.decode(&t).unwrap();
    return String::from_utf8(decoded).unwrap();
}
pub async fn add_song(
    names: &mut Vec<String>,
    urls: &mut Vec<String>,
    cur: usize,
    items: &Vec<Value>,
    index: String,
    _name: String,
    tx: Sender<QueueItem>,
) {
    let choice: usize = index.trim().parse().unwrap_or(0);
    if choice > 0 && choice <= items.len() {
        let track = &items[choice - 1];
        let id: i32 = track
            .get("id")
            .and_then(Value::as_i64)
            .map(|v| v as i32)
            .unwrap_or(0);
        let mut audio_quality: &str = track
            .get("audioQuality")
            .and_then(Value::as_str)
            .unwrap_or("LOSSLESS");
        let tags = track
            .get("mediaMetadata")
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_array)
            .unwrap();
        let qual = "HIRES_LOSSLESS";
        if tags.iter().any(|v| v.as_str() == Some(qual)) {
            audio_quality = "HI_RES_LOSSLESS";
        }

        let cached = check_song(&id.to_string());
        if cached {
            urls.insert(
                if cur == 0 { 0 } else { cur + 1 },
                concat_strings(Vec::from([
                    &env::var("HOME").unwrap(),
                    "/.local/share/mscply/songs/",
                    &id.to_string(),
                ])),
            );
            names.insert(
                if cur == 0 { 0 } else { cur + 1 },
                concat_strings(Vec::from([
                    track
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown"),
                    " - ",
                    track
                        .get("artist")
                        .and_then(|v| v.get("name").and_then(Value::as_str))
                        .unwrap_or("Unknown"),
                ])),
            )
        } else {
            let pref = PREF_QUAL.get().unwrap();
            if (audio_quality == "LOSSLESS" || audio_quality == "HI_RES_LOSSLESS")
                && (pref == "HIGH" || pref == "LOW")
            {
                audio_quality = pref;
            }
            let song = get_song(id, audio_quality).await.unwrap();
            let manifest = song
                .get("data")
                .and_then(|v| v.get("manifest"))
                .and_then(Value::as_str);
            let decoded = decode_base64(manifest.unwrap());
            if decoded.starts_with("<?xml") {
                urls.insert(if cur == 0 { 0 } else { cur + 1 }, decoded);
                names.insert(
                    if cur == 0 { 0 } else { cur + 1 },
                    concat_strings(Vec::from([
                        track
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("Unknown"),
                        " - ",
                        track
                            .get("artist")
                            .and_then(|v| v.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("Unknown"),
                    ])),
                );
            } else if let Ok(json) = serde_json::from_str::<Value>(&decoded) {
                if let Some(url) = json
                    .get("urls")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                {
                    urls.insert(if cur == 0 { 0 } else { cur + 1 }, url.to_string());
                    names.insert(
                        if cur == 0 { 0 } else { cur + 1 },
                        concat_strings(Vec::from([
                            track
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("Unknown"),
                            " - ",
                            track
                                .get("artist")
                                .and_then(|v| v.get("name"))
                                .and_then(Value::as_str)
                                .unwrap_or("Unknown"),
                        ])),
                    );
                }
            }
        }
    }
    spawn_recommendation_worker(
        items[choice - 1]
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string()
            + items[choice - 1]
                .get("artist")
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str)
                .unwrap(),
        tx,
    );
}
