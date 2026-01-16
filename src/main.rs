use base64::{Engine as _, engine::general_purpose};
use libmpv2::Mpv;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, REFERER, USER_AGENT};
use serde_json;
use serde_json::{Value, json};
use std::io::{self, Write, stdin, stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use std::{env, fs};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";
const QUERYBASE: &str = "https://triton.squid.wtf/search/?s=";
const STREAM: &str = "https://triton.squid.wtf/track/?";
static ID_CACHE: OnceLock<Mutex<Value>> = OnceLock::new();

fn global_json() -> &'static Mutex<Value> {
    ID_CACHE.get_or_init(|| {
        let path = format!(
            "{}/.local/share/mscply/cache.json",
            env::var("HOME").unwrap()
        );

        let value = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));

        Mutex::new(value)
    })
}

enum QueueItem {
    Url(String),
    Mpd(String),
}

async fn get_song(id: i32, audio_quality: &str) -> Result<Value, reqwest::Error> {
    let fin_url = format!("{}id={}&quality={}", STREAM, id, audio_quality);
    let client = reqwest::Client::new();
    let body: Value = client
        .get(fin_url)
        .header(USER_AGENT, AGENT)
        .send()
        .await?
        .json()
        .await?;
    Ok(body)
}

async fn search_result(query: &str) -> Result<Value, reqwest::Error> {
    let s: Vec<&str> = query.split(' ').collect();
    let q = format!("{}{}", QUERYBASE, s.join("%20"));
    let client = reqwest::Client::new();
    let body: Value = client
        .get(q)
        .header(USER_AGENT, AGENT)
        .send()
        .await?
        .json()
        .await?;
    Ok(body)
}

fn decode_base64(encoded: &str) -> String {
    let stripped = encoded.trim();
    let mut t = stripped.replace("-", "+").replace("_", "/");
    let missing = t.len() % 4;
    if missing == 1 {
        return String::from(stripped);
    } else if missing == 2 {
        t = format!("{}==", t);
    } else if missing == 3 {
        t = format!("{}=", t);
    }
    let decoded = general_purpose::STANDARD.decode(&t).unwrap();
    return String::from_utf8(decoded).unwrap();
}

fn queue_mpd_song(mpv: &mut Mpv, mpd: &str, init: i32) {
    use std::fs::OpenOptions;
    use std::io::Write;
    let path = format!("{}/mpd_file.mpd", env::temp_dir().display());
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{}", mpd).unwrap();
    f.flush().unwrap();
    queue_song(mpv, &path, init);
}

fn queue_song(mpv: &mut Mpv, url: &str, init: i32) {
    let idle: bool = mpv.get_property("idle-active").unwrap();

    // If mpv is idle, just start playing
    if idle {
        mpv.command("loadfile", &[url, "replace"]).unwrap();
        return;
    }

    // Get current playlist position
    let cur: i64 = mpv.get_property("playlist-pos").unwrap();

    // init == 1 → play next
    // init == 0 → append after the current insertion block
    let insert_pos = if init == 1 {
        cur + 1
    } else {
        // insert after the last queued item
        mpv.get_property::<i64>("playlist-count").unwrap()
    };

    mpv.command("loadfile", &[url, "insert-at", &insert_pos.to_string()])
        .unwrap();
}

async fn get_songlink_data(id: &str, source: &str) -> Value {
    let url = format!("https://song.link/{}/{}", source, id);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header(USER_AGENT, AGENT)
        .send()
        .await
        .unwrap();
    let re =
        regex::Regex::new(r#"<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
            .unwrap();
    let json_text = re.captures(&response.text().await.unwrap()).unwrap()[1].to_string();
    serde_json::from_str(&json_text).unwrap()
}

async fn convert_to_ytm(name: &str) -> Option<String> {
    let client = reqwest::Client::new();

    let body = json!({
        "context": {
            "client": {
                "hl": "en",
                "gl": "CA",
                "deviceMake": "",
                "deviceModel": "",
                "userAgent": AGENT,
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20260107.03.00",
                "osName": "X11",
                "osVersion": "",
                "originalUrl": "https://music.youtube.com/",
                "platform": "DESKTOP",
                "clientFormFactor": "UNKNOWN_FORM_FACTOR",
                "userInterfaceTheme": "USER_INTERFACE_THEME_DARK",
                "browserName": "Firefox",
                "browserVersion": "146.0",
                "acceptHeader": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                "screenWidthPoints": 1852,
                "screenHeightPoints": 661,
                "screenPixelDensity": 1,
                "screenDensityFloat": 1,
                "musicAppInfo": {
                    "pwaInstallabilityStatus": "PWA_INSTALLABILITY_STATUS_UNKNOWN",
                    "webDisplayMode": "WEB_DISPLAY_MODE_BROWSER",
                    "storeDigitalGoodsApiSupportStatus": {
                        "playStoreDigitalGoodsApiSupportStatus": "DIGITAL_GOODS_API_SUPPORT_STATUS_UNSUPPORTED"
                    }
                }
            },
            "user": { "lockedSafetyMode": false },
            "request": {
                "useSsl": true,
                "internalExperimentFlags": [],
                "consistencyTokenJars": []
            }
        },
        "query": name,
        "params": "EgWKAQIIAWoKEAMQBBAFEAoQCQ%3D%3D",
        "inlineSettingStatus": "INLINE_SETTING_STATUS_ON"
    });
    let new: Vec<&str> = name.trim().split(' ').collect();
    let res = client
        .post("https://music.youtube.com/youtubei/v1/search?prettyPrint=false")
        .header(USER_AGENT, AGENT)
        .header(CONTENT_TYPE, "application/json")
        .header(
            REFERER,
            format!("https://music.youtube.com/search?q={}", new.join("+")),
        )
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await;
    let resn = res.unwrap();
    let first: Option<String>;
    let arr = resn
        .get("contents")
        .and_then(|c| c.get("tabbedSearchResultsRenderer"))
        .and_then(|t| t.get("tabs"))
        .and_then(Value::as_array)
        .and_then(|tabs| tabs.get(0))
        .and_then(|tab| tab.get("tabRenderer"))
        .and_then(|tab| tab.get("content"))
        .and_then(|content| content.get("sectionListRenderer"))
        .and_then(|slr| slr.get("contents"))
        .and_then(Value::as_array)
        .unwrap();
    let zero_index = arr.get(0).unwrap();
    let first_index = arr.get(1);
    let con = zero_index.get("musicShelfRenderer");
    if !con.is_none() {
        first = con
            .unwrap()
            .get("contents")
            .and_then(Value::as_array)
            .and_then(|items| items.get(0))
            .and_then(|item| item.get("musicResponsiveListItemRenderer"))
            .and_then(|mr| mr.get("flexColumns"))
            .and_then(Value::as_array)
            .and_then(|cols| cols.get(0))
            .and_then(|col| col.get("musicResponsiveListItemFlexColumnRenderer"))
            .and_then(|flex| flex.get("text"))
            .and_then(|text| text.get("runs"))
            .and_then(Value::as_array)
            .and_then(|runs| runs.get(0))
            .and_then(|run| run.get("navigationEndpoint"))
            .and_then(|ne| ne.get("watchEndpoint"))
            .and_then(|we| we.get("videoId"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
    } else {
        first = first_index
            .unwrap()
            .get("musicShelfRenderer")
            .and_then(|msr| msr.get("contents"))
            .and_then(Value::as_array)
            .and_then(|items| items.get(0))
            .and_then(|item| item.get("musicResponsiveListItemRenderer"))
            .and_then(|mr| mr.get("flexColumns"))
            .and_then(Value::as_array)
            .and_then(|cols| cols.get(0))
            .and_then(|col| col.get("musicResponsiveListItemFlexColumnRenderer"))
            .and_then(|flex| flex.get("text"))
            .and_then(|text| text.get("runs"))
            .and_then(Value::as_array)
            .and_then(|runs| runs.get(0))
            .and_then(|run| run.get("navigationEndpoint"))
            .and_then(|ne| ne.get("watchEndpoint"))
            .and_then(|we| we.get("videoId"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
    }
    if first.is_none() {
        eprint!("{resn}");
    }
    first
}

async fn get_ytrecs(ytid: &str) -> Value {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0")
        .build()
        .unwrap();
    let body = json!({
        "enablePersistentPlaylistPanel": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        "videoId": format!("{}",ytid),
        "playlistId": format!("RDAMVM{}",ytid),
        "isAudioOnly": true,
        "responsiveSignals": {
            "videoInteraction": []
        },
        "queueContextParams": "",
        "context": {
            "client": {
                "hl": "en",
                "gl": "CA",
                "deviceMake": "",
                "deviceModel": "",
                "userAgent": AGENT,
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20260107.03.00",
                "osName": "X11",
                "osVersion": "",
                "originalUrl": format!("https://music.youtube.com/watch?v={}&list=RDAMVM{}", ytid, ytid),
                "platform": "DESKTOP",
                "clientFormFactor": "UNKNOWN_FORM_FACTOR",
                "userInterfaceTheme": "USER_INTERFACE_THEME_DARK",
                "acceptHeader": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                "screenWidthPoints": 1852,
                "screenHeightPoints": 661,
                "screenPixelDensity": 1,
                "screenDensityFloat": 1,
                "utcOffsetMinutes": -420,
                "musicAppInfo": {
                    "webDisplayMode": "WEB_DISPLAY_MODE_BROWSER",
                    "storeDigitalGoodsApiSupportStatus": {
                        "playStoreDigitalGoodsApiSupportStatus":
                            "DIGITAL_GOODS_API_SUPPORT_STATUS_UNSUPPORTED"
                    }
                }
            },
            "user": {
                "lockedSafetyMode": false
            },
            "request": {
                "useSsl": true,
                "internalExperimentFlags": [],
                "consistencyTokenJars": []
            }
        }
    });
    let resp = client
        .post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
        .header("Content-Type", "application/json")
        .header(USER_AGENT, AGENT)
        .header(
            "Referer",
            format!(
                "https://music.youtube.com/watch?v={}&list=RDAMVM{}",
                ytid, ytid
            ),
        )
        .json(&body)
        .send()
        .await
        .unwrap();
    resp.json::<Value>().await.unwrap()
}

fn get_ytrec_array(recs: Value) -> Vec<Value> {
    let tab0 = recs
        .get("contents")
        .and_then(|v| v.get("singleColumnMusicWatchNextResultsRenderer"))
        .and_then(|v| v.get("tabbedRenderer"))
        .and_then(|v| v.get("watchNextTabbedResultsRenderer"))
        .and_then(|v| v.get("tabs"))
        .and_then(Value::as_array)
        .and_then(|a| a.get(0))
        .unwrap();
    let cont = tab0
        .get("tabRenderer")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.get("musicQueueRenderer"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.get("playlistPanelRenderer"))
        .and_then(|v| v.get("contents"))
        .and_then(Value::as_array)
        .unwrap();
    let mut arr = Vec::new();
    for i in cont.iter().skip(1) {
        let id = i
            .get("playlistPanelVideoRenderer")
            .and_then(|v| v.get("videoId"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let name = i
            .get("playlistPanelVideoRenderer")
            .and_then(|v| v.get("title"))
            .and_then(|v| v.get("runs"))
            .and_then(Value::as_array)
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"));
        let jso = json!({"id": id, "name": name});
        arr.push(jso);
    }
    arr
}

fn extract_tidal_id(json: &Value) -> Option<String> {
    let sections = json
        .get("props")?
        .get("pageProps")?
        .get("pageData")?
        .get("sections")?
        .as_array()?;
    for section in sections {
        if let Some(links) = section.get("links").and_then(|l| l.as_array()) {
            for link in links {
                if link.get("platform")?.as_str()? == "tidal" {
                    if let Some(unique_id) = link.get("uniqueId")?.as_str() {
                        let parts: Vec<&str> = unique_id.split('|').collect();
                        if parts.len() == 3 {
                            return Some(parts[2].to_string());
                        }
                    }
                }
            }
        }
    }
    eprintln!("DEBUG::Tidal extraction failed :::: {json}");
    None
}

async fn get_quality(id: &str) -> String {
    let cli = Client::new();
    let res = cli
        .get(format!("https://hund.qqdl.site/info/?id={}", id.trim()))
        .header(USER_AGENT, AGENT)
        .header(REFERER, "https://tidal.squid.wtf/")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    // println!("{res} --- {id}");
    let qual = res
        .get("data")
        .and_then(|v| v.get("audioQuality"))
        .and_then(Value::as_str);
    if !qual.is_none() {
        let mut quality = qual.unwrap();
        let tags = res
            .get("data")
            .and_then(|v| v.get("mediaMetadata"))
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_array)
            .unwrap();
        if tags.iter().any(|v| v.as_str() == Some("HIRES_LOSSLESS")) {
            quality = "HI_RES_LOSSLESS";
        }
        quality.to_string()
    } else {
        "".to_string()
    }
}

async fn add_song(
    mpv: &mut Mpv,
    items: &Vec<Value>,
    index: String,
    name: String,
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
        let song = get_song(id, audio_quality).await.unwrap();
        let manifest = song
            .get("data")
            .and_then(|v| v.get("manifest"))
            .and_then(Value::as_str);
        let decoded = decode_base64(manifest.unwrap());
        if decoded.starts_with("<?xml") {
            queue_mpd_song(mpv, &decoded, 1);
        } else {
            if let Ok(json) = serde_json::from_str::<Value>(&decoded) {
                if let Some(urls) = json.get("urls").and_then(|v| v.as_array()) {
                    if let Some(first_url) = urls.first().and_then(Value::as_str) {
                        queue_song(mpv, first_url, 1);
                    } else {
                        println!("'urls' array is empty or first element is not a string");
                    }
                } else {
                    println!("No 'urls' array found");
                }
            }
        }
    }
    spawn_recommendation_worker(name, tx.clone());
}

fn spawn_input_thread(tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut buf = String::new();
        loop {
            print!(
                "Options: (p)ause, (r)esume, (h)alt, (f#)orward # secs, (s)kip, (v#)olume, (a)dd a song -> "
            );
            stdout().flush().unwrap();
            buf.clear();
            if io::stdin().read_line(&mut buf).is_ok() {
                let cmd = buf.trim().to_string();
                tx.send(cmd.clone().trim().to_string()).unwrap();
                if cmd == "a" {
                    let mut name = String::new();
                    stdin().read_line(&mut name).expect("");
                    tx.send(name.trim().to_string().clone()).unwrap();
                    if name.trim().eq_ignore_ascii_case("q") {
                        continue;
                    }
                    let mut index = String::new();
                    stdin().read_line(&mut index).expect("");
                    tx.send(index.trim().to_string()).unwrap();
                }
                if cmd == "h" {
                    SHUTDOWN.store(true, Ordering::SeqCst);
                    break;
                }
                // if tx.send(cmd.clone()).is_err() {
                //     break;
                // }
            }
        }
    });
}

fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut json = global_json().lock().unwrap();
        rt.block_on(async move {
            let new_iid = convert_to_ytm(&name).await.unwrap();

            let njson = get_ytrecs(&new_iid).await;
            let arr = get_ytrec_array(njson);

            for item in arr.iter() {
                if SHUTDOWN.load(Ordering::SeqCst) {
                    break;
                }
                let _name = item.get("name").and_then(Value::as_str).unwrap();
                let id = item.get("id").and_then(Value::as_str).unwrap();
                let tidal_id = json.get(id).and_then(Value::as_str);
                let tidal_id_final: String;
                if tidal_id.is_none() {
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
                    let quality = get_quality(&tidal_id_final).await;
                    if quality.is_empty() {
                        // eprintln!("Skipping {name} ({id})");
                        continue;
                    }
                    let id: i32 = tidal_id_final.parse().unwrap();
                    if let Ok(res) = get_song(id, &quality).await {
                        let manifest = res
                            .get("data")
                            .and_then(|d| d.get("manifest"))
                            .and_then(Value::as_str)
                            .unwrap();

                        let decoded = decode_base64(manifest);
                        // println!("WORKER: sending item");
                        if decoded.starts_with("<?xml") {
                            tx.send(QueueItem::Mpd(decoded)).ok();
                        } else if let Ok(json) = serde_json::from_str::<Value>(&decoded) {
                            if let Some(url) = json
                                .get("urls")
                                .and_then(|v| v.as_array())
                                .and_then(|a| a.first())
                                .and_then(Value::as_str)
                            {
                                tx.send(QueueItem::Url(url.to_string())).ok();
                            }
                        }
                    }
                } else {
                    // eprintln!("DEBUG::failed {name} - {id}");
                }
            }
        });
    });
}

fn save_cache() {
    let path = format!(
        "{}/.local/share/mscply/cache.json",
        env::var("HOME").unwrap()
    );

    fs::create_dir_all(path.rsplit_once('/').unwrap().0).unwrap();

    let json = global_json().lock().unwrap();
    fs::write(path, serde_json::to_string_pretty(&*json).unwrap()).unwrap();
}

#[tokio::main]
async fn main() {
    let (tx, rx): (Sender<QueueItem>, Receiver<QueueItem>) = mpsc::channel();
    let (input_tx, input_rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    let mut mpv = match Mpv::new() {
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
    spawn_input_thread(input_tx.clone());
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        while let Ok(item) = rx.try_recv() {
            match item {
                QueueItem::Url(url) => {
                    queue_song(&mut mpv, &url, 1);
                }
                QueueItem::Mpd(mpd) => {
                    queue_mpd_song(&mut mpv, &mpd, 1);
                }
            }
        }
        if let Ok(name) = input_rx.try_recv() {
            if name.eq_ignore_ascii_case("h") {
                break;
            } else if name.eq_ignore_ascii_case("p") {
                mpv.set_property("pause", true).unwrap();
            } else if name.starts_with("f") {
                if name.len() > 1 {
                    let val: i64 = match name[1..].parse() {
                        Ok(v) => v,
                        Err(_e) => {
                            continue;
                        }
                    };
                    let _ = mpv.command("seek", &[&format!("{}", val), "relative"]);
                }
            } else if name.eq_ignore_ascii_case("r") {
                mpv.set_property("pause", false).unwrap();
            } else if name.eq_ignore_ascii_case("s") {
                mpv.command("playlist-next", &["force"]).unwrap();
            } else if name.eq_ignore_ascii_case("a") {
                print!("Enter song name (q to exit): ");
                stdout().flush().unwrap();
                if let Ok(nammme) = input_rx.recv() {
                    if !nammme.is_empty() && !nammme.eq_ignore_ascii_case("q") {
                        let res = search_result(&nammme).await.unwrap();
                        let data = res
                            .get("data")
                            .and_then(|v| v.get("items"))
                            .and_then(Value::as_array)
                            .unwrap();
                        if data.is_empty() {
                            continue;
                        }
                        println!("\nResults:");
                        for (i, track) in data.iter().take(5).enumerate() {
                            println!(
                                "{}. {} - {}",
                                i + 1,
                                track
                                    .get("title")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Unknown"),
                                track
                                    .get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("Unknown")
                            )
                        }
                        print!("\nSelect track number: ");
                        stdout().flush().unwrap();
                        if let Ok(index) = input_rx.recv() {
                            add_song(&mut mpv, data, index, nammme, tx.clone()).await;
                        }
                    }
                }
            } else if name.starts_with("v") {
                if name.len() > 1 {
                    let val: i64 = match name[1..].parse() {
                        Ok(v) => v,
                        Err(_e) => {
                            continue;
                        }
                    };
                    mpv.set_property("volume", val).unwrap();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(tx);
    drop(input_tx);
    save_cache();
}
