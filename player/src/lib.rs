use libmpv2::Mpv;
use macros::*;
use network::*;
use rand::rng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::io::{Write, stderr};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio;

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ID_CACHE: OnceLock<Mutex<Value>> = OnceLock::new();
pub static SAVE_DATA: OnceLock<bool> = OnceLock::new();
pub static IS_RUNNING: AtomicBool = AtomicBool::new(false);
pub static CROSSFADE_DUR: f64 = 5.5;

pub enum QueueItem {
    Url(Value),
}

pub fn crossfade(mpv1: &mut Mpv, mpv2: &mut Mpv, new_song: &Value) {
    let cur_vol: f64 = mpv1.get_property("volume").unwrap();
    mpv2.set_property("volume", 0.0).unwrap();

    queue_song(mpv2, &new_song);

    let dur: f64 = mpv1.get_property("duration").unwrap();
    let start_time = Instant::now();

    while start_time.elapsed().as_secs_f64() < CROSSFADE_DUR {
        let prog = mpv1.get_property::<f64>("time-pos");
        if prog.is_err() {
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

        mpv1.set_property("volume", vol1).unwrap_or(());
        mpv2.set_property("volume", vol2).unwrap_or(());

        std::thread::sleep(Duration::from_millis(10));
    }

    // Ensure volumes are correct at the end
    mpv1.set_property("volume", 0.0).unwrap_or(());
    mpv2.set_property("volume", cur_vol).unwrap_or(());
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

pub fn save_index(json: &std::sync::MutexGuard<'_, Value>) {
    let path = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/songs/key_index.json",
    ]));
    fs::create_dir_all(path.rsplit_once('/').unwrap().0).unwrap();
    fs::write(path, serde_json::to_string_pretty(&**json).unwrap()).unwrap();
}

pub fn convert_to_v1(json: &std::sync::MutexGuard<'_, Value>) -> Value {
    let mut jsn = empty_json();
    jsn["JSONversion"] = json!("v1");
    if let Some(obj) = json.as_object() {
        for (i, k) in obj {
            jsn.as_object_mut()
                .unwrap()
                .insert(i.clone(), json!({"tidal": k.as_str().unwrap()}));
        }
    }
    jsn
}

pub fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut json = global_json().lock().unwrap_or_else(|e| e.into_inner());
            if json.get("JSONversion").and_then(Value::as_str).is_none(){
                let njson = convert_to_v1(&json);
                *json = njson;
            }
            IS_RUNNING.store(true, Ordering::SeqCst);
            let new_iid = convert_to_ytm(&name).await.unwrap();
            let njson = get_ytrecs(&new_iid).await;
            let mut arr = get_ytrec_array(njson);
            let arlen = arr.len();
            eprintln!("{arlen}");
            // arr.shuffle(&mut rng());
            let mut shuf = 0;
            while shuf < arlen {
                arr[shuf..std::cmp::min(arlen, shuf+10)].shuffle(&mut rng());
                shuf+=10;
            }
            stderr().flush().unwrap();
            let mut count = 0;
            for item in arr.iter() {
                if count > 5 {
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
                let qobuz_id = json.get(id).and_then(|v| v.get("qobuz")).and_then(Value::as_str);
                let qobuz_id_final: String;
                if qobuz_id.is_none() {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);

                        return;
                    }
                    // let songlink_data = get_songlink_data(id, "y").await;
                    let songs = search_result(&concat_strings(Vec::from([name, " ", artist]))).await.unwrap();
                    // let iiiid = extract_amazon_id(&songlink_data);
                    let iiiid = songs.get("items")
                    .and_then(Value::as_array)
                    .and_then(|v| v.first())
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_i64);
                    if iiiid.is_none() {
                        continue;
                    } else {
                        qobuz_id_final = iiiid.unwrap().to_string();
                        let entry = json
                            .as_object_mut()
                            .unwrap()
                            .entry(id.to_string())
                            .or_insert(json!({}));
                        if let Some(obj) = entry.as_object_mut() {
                            obj.insert("qobuz".to_string(), Value::String(qobuz_id_final.clone()));
                        }
                    }
                } else {
                    qobuz_id_final = qobuz_id.unwrap().to_string();
                }
                if !qobuz_id_final.is_empty() {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);

                        return;
                    }
                    let quality_json = get_quality(&qobuz_id_final).await;
                    let quality = quality_json.get("quality").and_then(Value::as_str).unwrap();
                    if quality.is_empty() {
                        continue;
                    }
                    let id = &qobuz_id_final;
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache(&json);
                        return;
                    }
                    if let Ok(res) = get_song(id, &quality).await {
                        let manifest = res
                            .get("directUrl")
                            .and_then(Value::as_str);
                        if !manifest.is_none() {
                            let url = manifest.unwrap();
                            if *SAVE_DATA.get().unwrap_or(&true) {
                                tx.send(QueueItem::Url(json!({"url":
                                    url.to_string(), "name":
                                    concat_strings(Vec::from([name, " - ", artist])),
                                    "id" : qobuz_id_final.to_string(),
                                    "duration":quality_json.get("duration").and_then(Value::as_i64).unwrap()})))
                                .ok();
                                continue;
                            }
                            cache_url(&qobuz_id_final, url).await;
                            tx.send(QueueItem::Url(json!({"url":
                                                        concat_strings(Vec::from([
                                                            &env::var("HOME").unwrap(),
                                                            "/.local/share/mscply/songs/",
                                                            &qobuz_id_final,
                                                        ])), "name":
                                                        concat_strings(Vec::from([name, " - ", artist])),
                                                        "id" : qobuz_id_final.to_string()})))
                            .ok();
                        }
                    }
                }
            }
            save_cache(&json);
            IS_RUNNING.store(false, Ordering::SeqCst);
        });
    });
}

pub fn queue_song(mpv: &mut Mpv, url: &Value) {
    let file_url = url.get("url").and_then(Value::as_str).unwrap();
    mpv.command("loadfile", &[file_url, "replace"]).unwrap();
}

pub async fn add_song(
    urls: &mut Vec<Value>,
    cur: usize,
    items: &Vec<Value>,
    index: String,
    _name: String,
    tx: Sender<QueueItem>,
) -> bool {
    let choice: usize = index.trim().parse().unwrap_or(0);
    if choice <= items.len() {
        let track = &items[choice];
        let id = track
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .to_string();
        let mut audio_quality = "flac";
        let title = track
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let artist = track
            .get("performer")
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let pref = PREF_QUAL.get().unwrap();
        if pref == "HIGH" {
            audio_quality = "opus";
        }
        let song = get_song(&id, audio_quality).await.unwrap();
        let manifest = song.get("directUrl").and_then(Value::as_str);
        if let Some(url) = manifest {
            urls.insert(
                if cur == 0 && urls.len() == 0 {
                    0
                } else {
                    cur + 1
                },
                json!({"url": url.to_string(), "name":concat_strings(Vec::from([
                        title,
                        " - ",
                        artist
                    ])), "id": id.to_string()
                    ,"duration": track
                        .get("duration")
                        .and_then(Value::as_i64).unwrap_or(0)}),
            );
        };
    }
    spawn_recommendation_worker(
        items[choice]
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string()
            + items[choice]
                .get("performer")
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str)
                .unwrap(),
        tx,
    );
    true
}
