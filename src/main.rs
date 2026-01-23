use base64::{Engine as _, engine::general_purpose};
use crossterm::{
    event::{Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use libmpv2::{self, Mpv};
use libmpv2::{
    Format,
    events::{Event as eve, PropertyData},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph},
};
use ratatui::{
    layout::{Alignment, Rect},
    widgets::{Clear, List, ListItem},
};
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, REFERER, USER_AGENT};
use serde_json;
use serde_json::{Value, json};
use std::io::Stdout;
use std::io::{Write, stdout};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, fs};
use tokio::time;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";
const QUERYBASE: &str = "https://tidal-api.binimum.org/search/?s=";
const STREAM: &str = "https://maus.qqdl.site/track/?";
const INFOSTREAM: &str = "triton.squid.wtf";
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
    Url(Vec<String>),
    Mpd(Vec<String>),
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

fn queue_mpd_song(mpv: &mut Mpv, mpd: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    let path = format!(
        "{}/mpd_{}.mpd",
        env::temp_dir().display(),
        uuid::Uuid::new_v4()
    );
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{}", mpd).unwrap();
    f.flush().unwrap();
    queue_song(mpv, &path);
}

fn queue_song(mpv: &mut Mpv, url: &str) {
    mpv.command("loadfile", &[url, "replace"]).unwrap();
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
        .get(format!("https://{}/info/?id={}", INFOSTREAM, id.trim()))
        .header(USER_AGENT, AGENT)
        .header(REFERER, "https://tidal.squid.wtf/")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
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
    names: &mut Vec<String>,
    urls: &mut Vec<String>,
    cur: usize,
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
        let cached = check_song(&format!("{}", id));
        if cached {
            urls.insert(
                if cur == 0 { 0 } else { cur + 1 },
                format!(
                    "{}/.local/share/mscply/songs/{}",
                    env::var("HOME").unwrap(),
                    id
                ),
            );
            names.insert(
                if cur == 0 { 0 } else { cur + 1 },
                track
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string(),
            )
        } else {
            let song = get_song(id, audio_quality).await.unwrap();
            let manifest = song
                .get("data")
                .and_then(|v| v.get("manifest"))
                .and_then(Value::as_str);
            let decoded = decode_base64(manifest.unwrap());
            urls.insert(if cur == 0 { 0 } else { cur + 1 }, decoded);
            names.insert(
                if cur == 0 { 0 } else { cur + 1 },
                track
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string(),
            );
        }
    }
    spawn_recommendation_worker(name, tx.clone());
}

fn spawn_recommendation_worker(name: String, tx: Sender<QueueItem>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut json = global_json().lock().unwrap();
            let new_iid = convert_to_ytm(&name).await.unwrap();

            let njson = get_ytrecs(&new_iid).await;
            let arr = get_ytrec_array(njson);
            let mut count = 0;
            for item in arr.iter() {
                if count > 10 {
                    count = 0;
                    save_cache();
                } else {
                    count += 1;
                }
                if SHUTDOWN.load(Ordering::SeqCst) {
                    save_cache();
                    return;
                }
                let name = item.get("name").and_then(Value::as_str).unwrap();
                let id = item.get("id").and_then(Value::as_str).unwrap();
                let tidal_id = json.get(id).and_then(Value::as_str);
                let tidal_id_final: String;
                if tidal_id.is_none() {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        save_cache();
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
                        save_cache();
                        return;
                    }
                    let cached = check_song(&tidal_id_final);
                    if cached {
                        tx.send(QueueItem::Url(Vec::from([
                            format!(
                                "{}/.local/share/mscply/songs/{}",
                                env::var("HOME").unwrap(),
                                tidal_id_final
                            ),
                            name.to_string(),
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
                        save_cache();
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
                                cache_mpd_song(&decoded, &tidal_id_final);
                                tx.send(QueueItem::Mpd(Vec::from([
                                    format!(
                                        "{}/.local/share/mscply/songs/{}",
                                        env::var("HOME").unwrap(),
                                        tidal_id_final
                                    ),
                                    name.to_string(),
                                ])))
                                .ok();
                            } else if let Ok(json) = serde_json::from_str::<Value>(&decoded) {
                                if let Some(url) = json
                                    .get("urls")
                                    .and_then(|v| v.as_array())
                                    .and_then(|a| a.first())
                                    .and_then(Value::as_str)
                                {
                                    cache_url(&tidal_id_final, url).await;
                                    tx.send(QueueItem::Url(Vec::from([
                                        format!(
                                            "{}/.local/share/mscply/songs/{}",
                                            env::var("HOME").unwrap(),
                                            tidal_id_final
                                        ),
                                        name.to_string(),
                                    ])))
                                    .ok();
                                }
                            }
                        }
                    }
                }
            }
            save_cache();
        });
    });
}

async fn cache_url(id: &str, url: &str) -> Option<String> {
    let path = format!(
        "{}/.local/share/mscply/songs/{}",
        env::var("HOME").unwrap(),
        id
    );
    if fs::metadata(&path).is_ok() {
        return Some(path);
    }

    let bytes = Client::new()
        .get(url)
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;

    fs::write(&path, bytes).ok()?;
    Some(path)
}

fn cache_mpd_song(mpd_string: &str, tidal_id: &str) {
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-protocol_whitelist",
            "file,pipe,https,tls,tcp,crypto",
            "-i",
            "pipe:0",
            "-f",
            "flac",
            "-v",
            "quiet",
            "-c",
            "copy",
            &format!(
                "{}/.local/share/mscply/songs/{}",
                env::var("HOME").unwrap(),
                tidal_id
            ),
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();

    ffmpeg
        .stdin
        .as_mut()
        .unwrap()
        .write_all(mpd_string.as_bytes())
        .unwrap();
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

fn check_song(id: &str) -> bool {
    let path = format!("{}/.local/share/mscply/songs/", env::var("HOME").unwrap());
    fs::create_dir_all(&path).unwrap();
    let f = fs::File::open(format!("{}/{}", path, id));
    if f.is_err() { false } else { true }
}

enum UiMode {
    Normal,
    Search,
    Results,
}

struct App {
    status: String,
    search_query: String,
    search_results: Vec<Value>,
    selected: usize,
    queue_len: i64,
    paused: bool,
    mode: UiMode,
    dirty: bool,
}

fn setup_terminal() -> Terminal<CrosstermBackend<Stdout>> {
    enable_raw_mode().unwrap();
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    Terminal::new(CrosstermBackend::new(stdout)).unwrap()
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();
}

fn draw_ui(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    let header = Paragraph::new("mscply — Tidal / YTM player")
        .block(Block::default().borders(Borders::ALL).title(""));

    let body = Paragraph::new(format!(
        "Status: {}\nPaused: {}\nQueue: {}",
        app.status, app.paused, app.queue_len
    ))
    .block(Block::default().borders(Borders::ALL).title("Player"));

    let footer = Paragraph::new(match app.mode {
        UiMode::Normal => "[a] Add  [p] Pause  [r] Resume  [s] Skip  [h] Quit",
        UiMode::Search => "Type search, Enter = search, Esc = cancel",
        UiMode::Results => "↑↓ select, Enter = add, Esc = cancel",
    })
    .block(Block::default().borders(Borders::ALL).title("Controls"));

    f.render_widget(header, chunks[0]);
    f.render_widget(body, chunks[1]);
    f.render_widget(footer, chunks[2]);
    if matches!(app.mode, UiMode::Search | UiMode::Results) {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);

        match app.mode {
            UiMode::Search => {
                let input = Paragraph::new(app.search_query.as_str())
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
                        ListItem::new(format!("{prefix}{title} — {artist}"))
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

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
        if !mpv.get_property::<bool>("idle-active").unwrap(){
            mpv.command("seek", &["100", "absolute-percent"]).unwrap();
        }
        return;
    }

    app.queue_len = (urls.len() - *current - 1) as i64;
    *current += 1;
    app.status = format!("Playing {}", names[*current]);

    if urls[*current].starts_with("<?xml") {
        queue_mpd_song(mpv, &urls[*current]);
    } else {
        queue_song(mpv, &urls[*current]);
    }

    app.dirty = true;
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let path = format!("{}/.local/share/mscply/songs/", env::var("HOME").unwrap());
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
            }
        }
    }
    let (tx, rx): (Sender<QueueItem>, Receiver<QueueItem>) = mpsc::channel();
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
    loop {
        if let Some(event) = mpv.wait_event(0.05) {
            match event.unwrap() {
                eve::EndFile(_) => {
                    if !skipped {
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
                        if !auto_started && current == 0 && !urls.is_empty() {
                            auto_started = true;
                            advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                        }
                    }
                }

                _ => {}
            }
        }
        if mpv.get_property::<bool>("idle-active").unwrap() && urls.len() >= current+1{
            advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
        }
        if app.dirty {
            terminal.draw(|f| draw_ui(f, &app)).unwrap();
            app.dirty = false;
        }
        while let Ok(item) = rx.try_recv() {
            match item {
                QueueItem::Url(url) => {
                    urls.push(url[0].clone());
                    names.push(url[1].clone());
                    app.queue_len += 1;
                }
                QueueItem::Mpd(mpd) => {
                    urls.push(mpd[0].clone());
                    names.push(mpd[1].clone());
                    app.queue_len += 1;
                }
            }
            app.dirty = true;
        }
        if crossterm::event::poll(time::Duration::from_millis(100)).unwrap() {
            if let Event::Key(key) = crossterm::event::read().unwrap() {
                if !key.is_press() {
                    continue;
                }
                match app.mode {
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
                            }
                        }
                        KeyCode::Char('r') => {
                            if app.paused {
                                mpv.set_property("pause", false).unwrap();
                                app.paused = false;
                                app.dirty = true;
                            }
                        }
                        KeyCode::Char('s') => {
                            if last_mode_switch.elapsed() >= skip_every {
                                skipped = true;
                                advance_playback(&mut mpv, &urls, &names, &mut current, &mut app);
                                last_mode_switch = Instant::now();
                            }
                        }
                        KeyCode::Char('a') => {
                            app.dirty = true;
                            app.search_query.clear();
                            app.mode = UiMode::Search;
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
                            let res = search_result(&app.search_query).await.unwrap();
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
                            app.queue_len += 1;
                            app.dirty = true;
                            if mpv.get_property::<i64>("playlist-pos").unwrap() == -1 {
                                app.status = format!("Playing {}", names[current]);
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
                }
            }
        }
    }
    restore_terminal(terminal);
}
