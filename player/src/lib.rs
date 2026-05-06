use libmpv2::Mpv;
use macros::*;
use network::*;
use rand::rng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio;

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ID_CACHE: OnceLock<Mutex<Value>> = OnceLock::new();
static INDEX_CACHE: OnceLock<Mutex<Value>> = OnceLock::new();
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

pub fn key_index() -> &'static Mutex<Value> {
    INDEX_CACHE.get_or_init(|| {
        let path = concat_strings(Vec::from([
            &env::var("HOME").unwrap(),
            "/.local/share/mscply/songs/key_index.json",
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

// pub fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
//     thread::spawn(move || {
//         let rt = tokio::runtime::Runtime::new().unwrap();
//         rt.block_on(async move {
//             let mut json = global_json().lock().unwrap_or_else(|e| e.into_inner());
//             if json.get("JSONversion").and_then(Value::as_str).is_none(){
//                 let njson = convert_to_v1(&json);
//                 *json = njson;
//             }
//             IS_RUNNING.store(true, Ordering::Relaxed);
//             let new_iid = convert_to_ytm(&name).await.unwrap();
//             let njson = get_ytrecs(&new_iid).await;
//             let mut arr = get_ytrec_array(njson);
//             let arlen = arr.len();
//             eprintln!("{arlen}");
//             // arr.shuffle(&mut rng());
//             let mut shuf = 0;
//             while shuf < arlen {
//                 arr[shuf..std::cmp::min(arlen, shuf+10)].shuffle(&mut rng());
//                 shuf+=10;
//             }
//             stderr().flush().unwrap();
//             let mut count = 0;
//             for item in arr.iter() {
//                 if count > 5 {
//                     save_cache(&json);
//                     count = 0;
//                 } else {
//                     count += 1;
//                 }
//                 if SHUTDOWN.load(Ordering::Relaxed) {
//                     save_cache(&json);
//                     return;
//                 }
//                 let name = item.get("name").and_then(Value::as_str).unwrap();
//                 let artist = item
//                     .get("artist")
//                     .and_then(Value::as_str)
//                     .unwrap_or("Unknown");
//                 let id = item.get("id").and_then(Value::as_str).unwrap();
//                 let mut fallback_used = false;
//                 let mut amazon_id = json.get(id).and_then(|v| v.get("amazon")).and_then(Value::as_str);
//                 let amazon_id_final: String;
//                 if amazon_id.is_none() {
//                     if SHUTDOWN.load(Ordering::Relaxed) {
//                         save_cache(&json);
//                         return;
//                     }
//                     // let songlink_data = get_songlink_data(id, "y").await;
//                     eprintln!("waiting 1");
//                     let mut songs = search_result(&concat_strings(Vec::from([name, " ", artist]))).await.unwrap();
//                     eprintln!("done 1");
//                     let mut iiiid = songs.get("results")
//                     .and_then(Value::as_array)
//                     .and_then(|v| v.first())
//                     .and_then(|v| v.get("hits"))
//                     .and_then(Value::as_array);
//                     if iiiid.is_none() {
//                         eprintln!("using fallback {name} {artist}");
//                         fallback_used = true;
//                         amazon_id = json.get(id).and_then(|v| v.get("qobuz")).and_then(Value::as_str);
//                         if amazon_id.is_none(){
//                             eprintln!("waiting 2");
//                             songs = fallback_search(&concat_strings(Vec::from([name, " ", artist]))).await.unwrap();
//                             eprintln!("done 2");

//                             iiiid = songs.get("items").and_then(Value::as_array);
//                             if iiiid.is_none() {
//                                 eprintln!("cant queue {name} {artist}");
//                                 continue;
//                             }
//                             if let Some(first) = iiiid.unwrap().first(){
//                                 if let Some(fin_id) = first.get("id").and_then(Value::as_i64){
//                                     amazon_id_final = fin_id.to_string();
//                                     let entry = json
//                                         .as_object_mut()
//                                         .unwrap()
//                                         .entry(id.to_string())
//                                         .or_insert(json!({}));
//                                     if let Some(obj) = entry.as_object_mut() {
//                                         obj.insert("qobuz".to_string(), Value::String(amazon_id_final.clone()));
//                                     }
//                                 }
//                                 else {
//                                     eprintln!("cant queue {name} {artist}");
//                                     continue;
//                                 }
//                             }
//                             else {
//                                 continue;
//                             }
//                         }
//                         else {
//                             amazon_id_final = amazon_id.unwrap().to_string();
//                         }
//                     } else {
//                         let final_song = iiiid.unwrap();
//                         // amazon_id_final = final_song[0].get("document").and_then(Value::as_str).unwrap().to_string();
//                         if let Some(e) = final_song.first(){
//                             if let Some(fin_track) = e.get("document").and_then(|v| v.get("asin")).and_then(Value::as_str){
//                                 amazon_id_final = fin_track.to_string();
//                                 let entry = json
//                                     .as_object_mut()
//                                     .unwrap()
//                                     .entry(id.to_string())
//                                     .or_insert(json!({}));
//                                 if let Some(obj) = entry.as_object_mut() {
//                                     obj.insert("amazon".to_string(), Value::String(amazon_id_final.clone()));
//                                 }
//                             }
//                             else {
//                                 continue;
//                             }
//                         }
//                         else {
//                             continue;
//                         }
//                     }
//                 } else {
//                     amazon_id_final = amazon_id.unwrap().to_string();
//                 }
//                 if !amazon_id_final.is_empty() {
//                     if SHUTDOWN.load(Ordering::Relaxed) {
//                         save_cache(&json);
//                         return;
//                     }
//                     let quality_json;
//                     if fallback_used {
//                         eprintln!("waiting 3");
//                         quality_json = fallback_metadata(&amazon_id_final).await;
//                         eprintln!("done 3");

//                     }
//                     else {
//                         eprintln!("waiting 4");
//                         quality_json = metadata(&amazon_id_final).await;
//                         eprintln!("done 4");
//                     }
//                     let quality = quality_json.get("quality").and_then(Value::as_str);
//                     if quality.is_none() {
//                         continue;
//                     }
//                     let id = &amazon_id_final;
//                     if SHUTDOWN.load(Ordering::Relaxed) {
//                         save_cache(&json);
//                         return;
//                     }
//                     if !fallback_used{
//                         eprintln!("waiting 5");
//                         if let Ok(res) = get_song(id, &quality.unwrap()).await {
//                             eprintln!("done 5");
//                             let manifest = res
//                                 .get("streamInfo")
//                                 .and_then(|d| d.get("streamUrl"))
//                                 .and_then(Value::as_str);
//                             if !manifest.is_none() {
//                                 let url = manifest.unwrap();
//                                 if *SAVE_DATA.get().unwrap_or(&true) {
//                                     tx.send(QueueItem::Url(json!({"url":
//                                         url.to_string(), "name":
//                                         concat_strings(Vec::from([name, " - ", artist])),
//                                         "key": res.get("decryptionKey").and_then(Value::as_str).unwrap_or(""),
//                                         "id" : amazon_id_final.to_string(),
//                                         "source": "amazon",
//                                         "duration":quality_json.get("duration").and_then(Value::as_i64).unwrap_or(0)})))
//                                     .ok();
//                                     continue;
//                                 }
//                                 eprintln!("waiting 6");
//                                 cache_url(&amazon_id_final, url).await;
//                                 eprintln!("done 6");
//                                 let key = res.get("decryptionKey").and_then(Value::as_str).unwrap_or("");
//                                 tx.send(QueueItem::Url(json!({"url":
//                                                             concat_strings(Vec::from([
//                                                                 &env::var("HOME").unwrap(),
//                                                                 "/.local/share/mscply/songs/",
//                                                                 &amazon_id_final,
//                                                             ])), "name":
//                                                             concat_strings(Vec::from([name, " - ", artist])),
//                                                             "key": key,
//                                                             "source": "amazon",
//                                                             "id" : amazon_id_final.to_string(),
//                                                             "duration":quality_json.get("duration").and_then(Value::as_i64).unwrap_or(0)
//                                 }))).ok();
//                             }
//                         }
//                     }
//                     else {
//                         eprintln!("waiting 7");
//                         if let Ok(res) = fallback_get_song(id, &quality.unwrap()).await {
//                             eprintln!("done 7");
//                             let manifest = res
//                                 .get("directUrl")
//                                 .and_then(Value::as_str);
//                             if !manifest.is_none() {
//                                 let url = manifest.unwrap();
//                                 if *SAVE_DATA.get().unwrap_or(&true) {
//                                     tx.send(QueueItem::Url(json!({"url":
//                                         url.to_string(), "name":
//                                         concat_strings(Vec::from([name, " - ", artist])),
//                                         "id" : amazon_id_final.to_string(),
//                                         "source": "qobuz",
//                                         "duration":quality_json.get("duration").and_then(Value::as_i64).unwrap()})))
//                                     .ok();
//                                     continue;
//                                 }
//                                 eprintln!("waiting 8");
//                                 cache_url(&amazon_id_final, url).await;
//                                 eprintln!("done 8");
//                                 tx.send(QueueItem::Url(json!({"url":
//                                 concat_strings(Vec::from([
//                                     &env::var("HOME").unwrap(),
//                                     "/.local/share/mscply/songs/",
//                                     &amazon_id_final,
//                                 ])), "name":
//                                 concat_strings(Vec::from([name, " - ", artist])),
//                                 "source": "qobuz",
//                                 "id" : amazon_id_final.to_string()})))
//                                 .ok();
//                             }
//                         }
//                     }
//                 }
//             }
//             save_cache(&json);
//             IS_RUNNING.store(false, Ordering::Relaxed);
//         });
//     });
// }

pub fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(run_worker(name, tx));
    });
}

fn get_cached_id(id: &str, source: &str) -> Option<String> {
    let json = global_json().lock().unwrap();
    if let Some(e) = json.get(id).and_then(|v| v.get(source)).and_then(Value::as_str){
        return Some(e.to_string())
    }
    None
}

async fn run_worker(name: String, tx: Sender<QueueItem>) {
    normalize_json_version();
    IS_RUNNING.store(true, Ordering::Relaxed);
    let new_iid = match convert_to_ytm(&name).await {
        Some(v) => v,
        None => return,
    };
    let recs = get_ytrecs(&new_iid).await;
    let mut arr = get_ytrec_array(recs);
    shuffle_in_chunks(&mut arr);
    let mut processed = 0;
    for item in arr {
        if should_shutdown() {
            save_global_cache();
            break;
        }
        if processed >= 10 {
            save_global_cache();
            processed = 0;
        }
        processed += 1;
        if let Some(track) = parse_track(&item) {
            if let Some(resolved) = resolve_track_id(&track).await {
                fetch_and_queue(&tx, &track, resolved).await;
            }
        }
    }

    save_global_cache();
    IS_RUNNING.store(false, Ordering::Relaxed);
}

#[derive(Debug)]
struct Track {
    name: String,
    artist: String,
    yt_id: String,
}

fn parse_track(v: &Value) -> Option<Track> {
    Some(Track {
        name: v.get("name")?.as_str()?.to_string(),
        artist: v.get("artist")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string(),
        yt_id: v.get("id")?.as_str()?.to_string(),
    })
}

struct ResolvedTrack {
    id: String,
    source: Source,
}

enum Source {
    Amazon,
    Qobuz,
}

fn extract_amazon_id(json: &Value) -> Option<String>{
    if let Some(e) = json.get("results").and_then(Value::as_array).and_then(|v| v.first()).and_then(|v| v.get("hits")).and_then(Value::as_array).and_then(|v| v.first()).and_then(|v| v.get("document")).and_then(|v| v.get("asin")).and_then(Value::as_str){
        return Some(e.to_string())
    }
    None
}

fn extract_qobuz_id(json: &Value) -> Option<String>{
    if let Some(e) = json.get("items").and_then(Value::as_array).and_then(|v| v.first()).and_then(|v| v.get("id")).and_then(Value::as_str){
        return Some(e.to_string())
    }
    None
}

fn cache_id(ytid: &str, source: &str, id: &str){
    let mut json = global_json().lock().unwrap_or_else(|e| e.into_inner());
    let entry = json
                            .as_object_mut()
                            .unwrap()
                            .entry(ytid.to_string())
                            .or_insert(json!({}));
    if let Some(obj) = entry.as_object_mut() {
                                            obj.insert(source.to_string(), Value::String(id.to_string()));
                                        }
}

async fn resolve_track_id(track: &Track) -> Option<ResolvedTrack> {
    // try cache first
    if let Some(id) = get_cached_id(&track.yt_id, "amazon") {
        return Some(ResolvedTrack { id, source: Source::Amazon });
    }

    let query = format!("{} {}", track.name, track.artist);

    // primary search
    if let Ok(songs) = search_result(&query).await {
        if let Some(id) = extract_amazon_id(&songs) {
            cache_id(&track.yt_id, "amazon", &id);
            return Some(ResolvedTrack { id, source: Source::Amazon });
        }
    }

    // fallback
    if let Some(id) = get_cached_id(&track.yt_id, "qobuz") {
        return Some(ResolvedTrack { id, source: Source::Qobuz });
    }

    if let Ok(songs) = fallback_search(&query).await {
        if let Some(id) = extract_qobuz_id(&songs) {
            cache_id(&track.yt_id, "qobuz", &id);
            return Some(ResolvedTrack { id, source: Source::Qobuz });
        }
    }
    None
}

async fn fetch_and_queue(tx: &Sender<QueueItem>, track: &Track, resolved: ResolvedTrack) {
    if should_shutdown() {
        return;
    }

    let quality_json = match resolved.source {
        Source::Amazon => metadata(&resolved.id).await,
        Source::Qobuz => fallback_metadata(&resolved.id).await,
    };

    let quality = match quality_json.get("quality").and_then(Value::as_str) {
        Some(q) => q,
        None => return,
    };

    match resolved.source {
        Source::Amazon => queue_amazon(tx, track, &resolved.id, quality, &quality_json).await,
        Source::Qobuz => queue_qobuz(tx, track, &resolved.id, quality, &quality_json).await,
    }
}

async fn queue_amazon(
    tx: &Sender<QueueItem>,
    track: &Track,
    id: &str,
    quality: &str,
    meta: &Value,
) {
    if let Ok(res) = get_song(id, quality).await {
        if let Some(url) = res.get("streamInfo")
            .and_then(|v| v.get("streamUrl"))
            .and_then(Value::as_str)
        {
            let key = res.get("decryptionKey").and_then(Value::as_str).unwrap_or("");

            send_to_queue(tx, track, id, url, key, "amazon", meta);
        }
    }
}

async fn queue_qobuz(
    tx: &Sender<QueueItem>,
    track: &Track,
    id: &str,
    quality: &str,
    meta: &Value,
) {
    if let Ok(res) = fallback_get_song(id, quality).await {
        if let Some(url) = res.get("directUrl").and_then(Value::as_str) {
            send_to_queue(tx, track, id, url, "", "qobuz", meta);
        }
    }
}

fn send_to_queue(
    tx: &Sender<QueueItem>,
    track: &Track,
    id: &str,
    url: &str,
    key: &str,
    source: &str,
    meta: &Value,
) {
    let payload = json!({
        "url": url,
        "name": format!("{} - {}", track.name, track.artist),
        "key": key,
        "id": id,
        "source": source,
        "duration": meta.get("duration").and_then(Value::as_i64).unwrap_or(0)
    });

    let _ = tx.send(QueueItem::Url(payload));
}

fn should_shutdown() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

fn shuffle_in_chunks(arr: &mut [Value]) {
    let mut shuf = 0;
    let arlen = arr.len();
    while shuf < arlen {
        arr[shuf..std::cmp::min(arlen, shuf+10)].shuffle(&mut rng());
        shuf+=10;
    }
}

fn normalize_json_version() {
    let mut json = global_json().lock().unwrap();
    if json.get("JSONversion").and_then(Value::as_str).is_none() {
        *json = convert_to_v1(&json);
    }
}

fn save_global_cache() {
    let json = global_json().lock().unwrap();
    save_cache(&json);
}

pub fn queue_song(mpv: &mut Mpv, url: &Value) {
    let file_url = url.get("url").and_then(Value::as_str).unwrap();
    let key = url.get("key").and_then(Value::as_str).unwrap_or("");
    if !key.is_empty() {
        mpv.set_property(
            "demuxer-lavf-o",
            concat_strings(Vec::from(["decryption_key=", key])),
        )
        .unwrap();
    }
    mpv.command("loadfile", &[file_url, "replace"]).unwrap();
}

fn is_streamable(json: &Value) -> bool {
    json.get("stremeable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
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
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut audio_quality = "flac";
        let title = track
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let artist = track
            .get("artist")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let source = track.get("source").and_then(Value::as_str).unwrap();
        let pref = PREF_QUAL.get().unwrap();
        if pref == "HIGH" {
            audio_quality = "opus";
        }
        let song;
        if source == "amazon"{
            song = get_song(id, audio_quality).await.unwrap();
            if !is_streamable(&song){
                return false;
            }
            let key = song.get("decryptionKey").and_then(Value::as_str).unwrap();
            let manifest = song
                .get("streamInfo")
                .and_then(|d| d.get("streamUrl"))
                .and_then(Value::as_str);
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
                    ])), "id": id.to_string(),
                    "key": key,
                    "source": source
                    ,"duration": track
                        .get("duration").and_then(Value::as_i64).unwrap_or(0)}),
                );
            }
        }
        else {
            song = fallback_get_song(id, audio_quality).await.unwrap();
            if let Some(manifest) = song
                .get("directUrl")
                .and_then(Value::as_str){
                    urls.insert(
                        if cur == 0 && urls.len() == 0 {
                            0
                        } else {
                            cur + 1
                        },
                        json!({"url": manifest.to_string(), "name":concat_strings(Vec::from([
                            title,
                            " - ",
                            artist
                        ])), "id": id.to_string(),
                        "source": source
                        ,"duration": track
                            .get("duration").and_then(Value::as_i64).unwrap_or(0)}),
                    );
                }

        }

    }
    spawn_recommendation_worker(
        items[choice]
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string()
            + items[choice]
                .get("artist")
                .and_then(Value::as_str)
                .unwrap(),
        tx,
    );
    true
}
