use base64::{Engine as _, engine::general_purpose};
use macros::*;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, COOKIE, REFERER, USER_AGENT};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, UNIX_EPOCH};
use std::{env, fs};
use url;
use uuid;

pub static PREF_QUAL: OnceLock<String> = OnceLock::new();
pub static INFOSTREAM: AtomicBool = AtomicBool::new(false);
static CLIENT: OnceLock<Client> = OnceLock::new();
pub static IS_CACHING: AtomicBool = AtomicBool::new(false);
pub const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:149.0) Gecko/20100101 Firefox/149.0";
pub static API: &str = "https://t2tunes.site/api/amazon-music";
static LAST_CHALLENGE: AtomicI64 = AtomicI64::new(0);
pub static FALLBACK: &str = "https://qobuz.squid.wtf";
static SUGGESTION_SOURCE: &str = "https://spotiflac.eclipsemusic.app/9fce354c40f3cbf0/";

pub struct CacheItem {
    pub path: String,
    pub index: usize,
}

use sha2::{Digest, Sha256};

// ----------------------------
// helpers
// ----------------------------

fn hex_to_bytes(hex_str: &str) -> Vec<u8> {
    hex::decode(hex_str).expect("Invalid hex string")
}

fn buffer_starts_with(buf: &[u8], prefix: &[u8]) -> bool {
    buf.starts_with(prefix)
}

fn buffer_to_hex(buf: &[u8]) -> String {
    hex::encode(buf)
}

// ----------------------------
// password buffer
// ----------------------------

struct PasswordBuffer {
    nonce: Vec<u8>,
    buffer: Vec<u8>,
}

impl PasswordBuffer {
    fn new(nonce: Vec<u8>) -> Self {
        let mut buffer = vec![0u8; nonce.len() + 4];
        buffer[..nonce.len()].copy_from_slice(&nonce);

        Self { nonce, buffer }
    }

    fn set_counter(&mut self, counter: u32) -> Vec<u8> {
        let start = self.nonce.len();
        self.buffer[start..].copy_from_slice(&counter.to_be_bytes());
        self.buffer.clone()
    }
}

// ----------------------------
// derive_key
// ----------------------------

fn derive_key(
    algorithm: &str,
    salt: &[u8],
    password: &[u8],
    cost: u32,
    key_length: usize,
) -> Vec<u8> {
    let mut derived: Vec<u8> = Vec::new();

    for i in 0..std::cmp::max(1, cost) {
        let data = if i == 0 {
            [salt, password].concat()
        } else {
            derived.clone()
        };

        let hash = match algorithm.to_lowercase().as_str() {
            "sha-256" | "sha256" => {
                let mut hasher = Sha256::new();
                hasher.update(&data);
                hasher.finalize().to_vec()
            }
            _ => panic!("Only SHA-256 supported"),
        };

        derived = hash[..key_length].to_vec();
    }

    derived
}

// ----------------------------
// parameters struct
// ----------------------------

#[derive(Debug, Deserialize)]
struct Parameters {
    nonce: String,
    salt: String,
    #[serde(rename = "keyPrefix")]
    key_prefix: String,
    cost: u32,
    #[serde(rename = "keyLength")]
    key_length: Option<usize>,
    algorithm: Option<String>,
}

// ----------------------------
// solve_challenge
// ----------------------------

fn solve_challenge(parameters: Parameters) -> Option<serde_json::Value> {
    let counter_start = 0;
    let counter_step = 1;
    let timeout_secs = 10;
    let nonce = hex_to_bytes(&parameters.nonce);
    let salt = hex_to_bytes(&parameters.salt);
    let key_prefix = parameters.key_prefix;
    let cost = parameters.cost;
    let key_length = parameters.key_length.unwrap_or(32);
    let algorithm = parameters
        .algorithm
        .unwrap_or_else(|| "SHA-256".to_string());

    let key_prefix_bytes = if key_prefix.len() % 2 == 0 {
        Some(hex_to_bytes(&key_prefix))
    } else {
        None
    };

    let mut password = PasswordBuffer::new(nonce);

    let start = Instant::now();
    let mut counter = counter_start;

    loop {
        if start.elapsed() > Duration::from_secs(timeout_secs) {
            return None;
        }

        let pwd = password.set_counter(counter);

        let derived = derive_key(&algorithm, &salt, &pwd, cost, key_length);

        let ok = if let Some(prefix_bytes) = &key_prefix_bytes {
            buffer_starts_with(&derived, prefix_bytes)
        } else {
            buffer_to_hex(&derived).starts_with(&key_prefix)
        };

        if ok {
            return Some(json!({
                "counter": counter,
                "derivedKey": buffer_to_hex(&derived),
                "time": start.elapsed().as_secs_f64() * 1000.0
            }));
        }

        counter = counter.wrapping_add(counter_step);
    }
}

fn get_time() -> i64 {
    let mil_time = std::time::SystemTime::now();
    (mil_time
        .duration_since(UNIX_EPOCH)
        .expect("fkin time????")
        .as_secs_f64()
        * 1000.0) as i64
}

async fn fetch_challenge() -> Result<String, reqwest::Error> {
    let epoch_time = get_time();
    let client = CLIENT.get().unwrap().clone();
    let url = format!("{FALLBACK}/api/altcha/challenge?ts={epoch_time}");
    let res = client
        .get(url)
        .header(USER_AGENT, AGENT)
        .header(REFERER, FALLBACK)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(res)
}

fn make_payload(final_json: &str) -> String {
    let encoded = general_purpose::STANDARD.encode(final_json);
    format!("{{\"payload\" : \"{encoded}\"}}")
}

async fn post_payload(json: String) -> Result<(), reqwest::Error> {
    let client = CLIENT.get().unwrap().clone();
    let url = format!("{FALLBACK}/api/altcha/verify");
    let _ = client
        .post(url)
        .header(REFERER, FALLBACK)
        .header(USER_AGENT, AGENT)
        .header(CONTENT_TYPE, "application/json")
        .body(json)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn update_challenge() -> bool {
    let mut challenge_json = None;
    while challenge_json.is_none() {
        let mut challenge = fetch_challenge().await;
        while challenge.is_err() {
            challenge = fetch_challenge().await;
        }
        let challenge_string = challenge.unwrap();
        let chall_json = serde_json::from_str::<Value>(&challenge_string);
        if chall_json.is_err() {
            continue;
        }
        challenge_json = Some(chall_json.unwrap());
    }
    let final_challenge = challenge_json.unwrap();
    if let Some(params) = final_challenge.get("parameters") {
        let solved = solve_challenge(serde_json::from_value::<Parameters>(params.clone()).unwrap());
        if solved.is_none() {
            return false;
        } else {
            let payload_json =
                json!({"challenge": final_challenge, "solution":solved.unwrap()}).to_string();
            let payload_string = make_payload(&payload_json);
            let t = get_time();
            if post_payload(payload_string).await.is_err() {
                return false;
            }
            LAST_CHALLENGE.store(t, std::sync::atomic::Ordering::Relaxed);
            return true;
        }
    } else {
        false
    }
}

pub fn infostream() -> bool {
    let d = INFOSTREAM.load(std::sync::atomic::Ordering::Relaxed);
    return d.clone();
}

pub async fn fallback_metadata(qobuz_id: &str) -> Value {
    let cli = CLIENT.get().unwrap().clone();
    let url = concat_strings(Vec::from([
        FALLBACK,
        "/api/get-music?offset=0&q=",
        qobuz_id,
    ]));
    let mut jsn = empty_json();
    let resp = cli
        .get(url)
        .header(USER_AGENT, AGENT)
        .header(REFERER, FALLBACK)
        .timeout(Duration::from_secs(7))
        .send()
        .await;
    let res = match resp {
        Ok(e) => e.error_for_status(),
        Err(_) => {
            return jsn;
        }
    };
    if res.is_err() {
        return jsn;
    }
    let resp = res.unwrap().text().await.unwrap();
    if let Ok(e) = serde_json::from_str::<Value>(&resp) {
        if let Some(m) = e
            .get("data")
            .and_then(|v| v.get("tracks"))
            .and_then(|v| v.get("items"))
            .and_then(Value::as_array)
            .and_then(|v| v.get(0))
        {
            if let Some(bit) = m.get("maximum_bit_depth").and_then(Value::as_i64) {
                if bit >= 16 {
                    jsn.as_object_mut()
                        .unwrap()
                        .insert("quality".to_string(), json!("flac"));
                } else {
                    jsn.as_object_mut()
                        .unwrap()
                        .insert("quality".to_string(), json!("opus"));
                }
            }
            if let Some(dur) = m.get("duration").and_then(Value::as_i64) {
                jsn.as_object_mut()
                    .unwrap()
                    .insert("duration".to_string(), json!(dur));
            }
        }
    }
    jsn
}

pub async fn metadata(id: &str) -> Value {
    let cli = CLIENT.get().unwrap().clone();
    let url = concat_strings(Vec::from([API, "/metadata?asin=", id]));
    let mut resp = None;
    let res = cli
        .get(url)
        .header(USER_AGENT, AGENT)
        .timeout(Duration::from_secs(7))
        .send()
        .await;
    let response = match res {
        Ok(e) => e.error_for_status(),
        Err(_) => {
            return empty_json();
        }
    };
    if !response.is_err() {
        resp = Some(response.unwrap().json::<Value>().await.expect("JSON ERROR"));
    }
    if resp.is_none() {
        return empty_json();
    }
    let res = resp.expect("Status error");
    let mut qual = None;
    if let Some(qual_arr) = res
        .get("trackList")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .and_then(|v| v.get("assetQualities"))
        .and_then(Value::as_array)
    {
        for i in qual_arr {
            if i.get("quality").and_then(Value::as_str) == Some("CD") {
                qual = Some("flac");
                break;
            } else {
                qual = Some("opus");
            }
        }
    }
    let mut duration = None;
    if let Some(dur) = res
        .get("trackList")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .and_then(|v| v.get("duration"))
        .and_then(Value::as_i64)
    {
        duration = Some(dur);
    }
    let pref = PREF_QUAL.get().unwrap();
    if !qual.is_none() {
        if qual == Some("flac") && pref == "HIGH" {
            return json!({"quality":"opus", "duration": duration.unwrap_or(0)});
        }
        json!({"quality":qual.unwrap_or("opus"), "duration": duration.unwrap_or(0)})
    } else {
        json!({"quality": "opus"})
    }
}

pub fn cache_next_song(url: String, index: usize, sx: Sender<CacheItem>) {
    tokio::spawn(async move {
        let path = concat_strings(Vec::from([
            env::temp_dir().to_str().unwrap(),
            "/",
            &uuid::Uuid::new_v4().to_string(),
        ]));
        let path2 = path.to_string();
        let return_item = CacheItem {
            path: path2,
            index: index,
        };
        IS_CACHING.store(true, std::sync::atomic::Ordering::SeqCst);
        let cli = Client::new();

        if url.starts_with("<?xml") {
            let new: Vec<&str> = url.split(" ").collect();
            let mut init: String = "--".to_string();
            let mut r = "--";
            for i in new {
                if i.starts_with("media=") {
                    init = i[7..i.len() - 1].to_string();
                } else if i.starts_with("r=") {
                    let smth = i.split("/").collect::<Vec<&str>>()[0];
                    r = &smth[3..smth.len() - 1];
                }
            }
            init = init.replace("amp;", "");
            let new_init = init.split("$Number$").collect::<Vec<&str>>();
            let r = r.parse::<u32>().unwrap();
            let mut bytes: Vec<u8> = Vec::new();
            for i in 0..=r + 2 {
                let resp = cli
                    .get(concat_strings(Vec::from([
                        new_init[0],
                        &i.to_string(),
                        new_init[1],
                    ])))
                    .send()
                    .await;
                if resp.is_err() {
                    IS_CACHING.store(false, std::sync::atomic::Ordering::SeqCst);
                    sx.send(return_item).unwrap();
                    return;
                }
                let chunk = resp.unwrap().bytes().await.unwrap();
                bytes.extend_from_slice(&chunk);
            }
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&path)
                .unwrap();
            handle.write_all(&bytes).unwrap();
            IS_CACHING.store(false, std::sync::atomic::Ordering::SeqCst);
            sx.send(return_item).unwrap();
            return;
        } else {
            let bytes = cli.get(url).send().await.unwrap().bytes().await;
            fs::write(&path, bytes.unwrap()).ok();
            IS_CACHING.store(false, std::sync::atomic::Ordering::SeqCst);
            sx.send(return_item).unwrap();
            return;
        }
    });
}

pub fn set_url() {
    tokio::spawn(async {
        let cli = reqwest::Client::new();
        CLIENT.set(cli.clone()).unwrap();
        let mut last_update = Instant::now();
        let mut res = update_challenge().await;
        while !res {
            res = update_challenge().await;
        }
        loop {
            if last_update.elapsed() >= Duration::from_mins(15) {
                let res = update_challenge().await;
                if res {
                    last_update = Instant::now();
                }
            }
            let res = cli
                .get("https://t2tunes.site/api/status")
                .send()
                .await
                .unwrap();
            let jsn = res.json::<Value>().await.unwrap();
            if jsn.get("amazonMusic").and_then(Value::as_str) == Some("up") {
                INFOSTREAM.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                INFOSTREAM.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_mins(1)).await;
        }
    });
}

pub async fn get_song(id: &str, audio_quality: &str) -> Result<Value, reqwest::Error> {
    let fin_url = concat_strings(Vec::from([
        API,
        "/media-from-asin?asin=",
        id,
        "&codec=",
        audio_quality,
    ]));
    let client = CLIENT.get().unwrap().clone();
    let mut body = None;
    if let Ok(b) = client
        .get(&fin_url)
        .header(USER_AGENT, AGENT)
        .header(REFERER, API)
        .send()
        .await
    {
        if let Ok(jsn) = b.error_for_status()?.json::<Value>().await {
            if let Some(element) = jsn.as_array() {
                if let Some(first) = element.first() {
                    body = Some(first.clone())
                }
            }
        }
    }

    if body.is_none() {
        Ok(empty_json())
    } else {
        Ok(body.unwrap())
    }
}

pub async fn fallback_get_song(
    qobuz_id: &str,
    audio_quality: &str,
) -> Result<Value, reqwest::Error> {
    let fin_url = concat_strings(Vec::from([
        FALLBACK,
        "/api/download-music?track_id=",
        qobuz_id,
        "&format_id=",
        if audio_quality == "flac" { "27" } else { "5" },
    ]));
    let client = CLIENT.get().unwrap().clone();

    let mut body = None;

    if let Ok(b) = client
        .get(&fin_url)
        .header(USER_AGENT, AGENT)
        .header(
            COOKIE,
            format!(
                "captcha_verified_at={}",
                LAST_CHALLENGE
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .to_string()
            ),
        )
        .header(REFERER, FALLBACK)
        .send()
        .await
    {
        if let Ok(jsn) = b.error_for_status()?.json::<Value>().await {
            body = Some(jsn);
        }
    }

    if body.is_none() {
        Ok(empty_json())
    } else {
        Ok(body.unwrap())
    }
}

pub async fn search_result(query: &str) -> Result<Value, reqwest::Error> {
    // let s = query
    //     .split(' ')
    //     .collect::<Vec<&str>>()
    //     .join("+")
    //     .to_string();
    // let q = concat_strings(Vec::from([
    //     API,
    //     "/search?query=",
    //     s.as_str(),
    //     "&types=track",
    // ]));
    let mut q = url::Url::parse(API).unwrap();
    q.path_segments_mut().unwrap().push("search");
    q.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("types", "track");
    let client = CLIENT.get().unwrap().clone();
    let mut body = None;
    if let Ok(b) = client
        .get(q)
        .header(USER_AGENT, AGENT)
        .header(REFERER, API)
        .send()
        .await
    {
        if let Ok(e) = b.error_for_status()?.json::<Value>().await {
            body = Some(e)
        }
    }

    if body.is_none() {
        return Ok(empty_json());
    }
    let body = body.unwrap();
    Ok(body)
}

pub async fn fallback_search(query: &str) -> Result<Value, reqwest::Error> {
    let mut q = url::Url::parse(FALLBACK).unwrap();
    q.path_segments_mut().unwrap().push("api");
    q.path_segments_mut().unwrap().push("get-music");
    q.query_pairs_mut().append_pair("offset", "0");
    q.query_pairs_mut().append_pair("q", query);

    let q = q.to_string().replace("+", "%20");
    let client = CLIENT.get().unwrap().clone();

    let mut body = None;
    if let Ok(b) = client.get(q).header(USER_AGENT, AGENT).send().await {
        if let Ok(e) = b.error_for_status()?.text().await {
            if let Ok(data) = serde_json::from_str::<Value>(&e) {
                if let Some(j) = data.get("data").and_then(|v| v.get("tracks")) {
                    body = Some(j.clone());
                }
            }
        }
    }
    if body.is_none() {
        return Ok(empty_json());
    }
    let body = body.unwrap();
    Ok(body)
}

pub async fn convert_to_ytm(name: &str) -> Option<String> {
    let client = CLIENT.get().unwrap().clone();

    let body = query_json(name);
    let new: Vec<&str> = name.trim().split(' ').collect();
    let mut res = client
        .post("https://music.youtube.com/youtubei/v1/search?prettyPrint=false")
        .header(USER_AGENT, AGENT)
        .header(CONTENT_TYPE, "application/json")
        .header(
            REFERER,
            concat_strings(Vec::from([
                "https://music.youtube.com/search?q=",
                new.join("+").as_str(),
            ])),
        )
        .timeout(Duration::from_secs(7))
        .json(&body)
        .send()
        .await;
    while res.is_err() {
        res = client
            .post("https://music.youtube.com/youtubei/v1/search?prettyPrint=false")
            .header(USER_AGENT, AGENT)
            .header(CONTENT_TYPE, "application/json")
            .header(
                REFERER,
                concat_strings(Vec::from([
                    "https://music.youtube.com/search?q=",
                    new.join("+").as_str(),
                ])),
            )
            .json(&body)
            .send()
            .await;
    }
    let ress = res.unwrap().json::<Value>().await;
    if ress.is_err() {
        return Some("".to_string());
    }
    let resn = ress.unwrap();
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
    if !first.is_none() {
        first
    } else {
        Some("".to_string())
    }
}

pub async fn get_ytrecs(ytid: &str) -> Value {
    if ytid.is_empty() {
        return empty_json();
    }
    let client = CLIENT.get().unwrap().clone();
    let body = ytrecs_json(ytid);
    let resp = client
        .post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
        .header("Content-Type", "application/json")
        .header(USER_AGENT, AGENT)
        .header(
            "Referer",
            concat_strings(Vec::from([
                "https://music.youtube.com/watch?v=",
                ytid,
                "&list=RDAMVM",
                ytid,
            ])),
        )
        .timeout(Duration::from_secs(10))
        .json(&body)
        .send()
        .await;
    let mut res = None;
    while resp.is_err() || res.is_none() {
        let resp = client
            .post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
            .header("Content-Type", "application/json")
            .header(USER_AGENT, AGENT)
            .header(
                "Referer",
                concat_strings(Vec::from([
                    "https://music.youtube.com/watch?v=",
                    ytid,
                    "&list=RDAMVM",
                    ytid,
                ])),
            )
            .json(&body)
            .send()
            .await;
        if !resp.is_err() {
            res = Some(resp.unwrap().text().await.unwrap());
        }
    }
    serde_json::from_str(&res.unwrap()).unwrap()
}

pub async fn cache_url(id: &str, url: &str) -> Option<String> {
    let path = concat_strings(Vec::from([
        &env::var("HOME").unwrap(),
        "/.local/share/mscply/songs/",
        id,
    ]));
    if fs::metadata(&path).is_ok() {
        return Some(path);
    }

    let bytes = CLIENT
        .get()
        .unwrap()
        .clone()
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

pub fn get_ytrec_array(recs: Value) -> Option<Vec<Value>> {
    let tab0 = recs
        .get("contents")
        .and_then(|v| v.get("singleColumnMusicWatchNextResultsRenderer"))
        .and_then(|v| v.get("tabbedRenderer"))
        .and_then(|v| v.get("watchNextTabbedResultsRenderer"))
        .and_then(|v| v.get("tabs"))
        .and_then(Value::as_array)
        .and_then(|a| a.get(0))
        .cloned()
        .unwrap_or(empty_json());
    let cont = tab0
        .get("tabRenderer")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.get("musicQueueRenderer"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.get("playlistPanelRenderer"))
        .and_then(|v| v.get("contents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or(Vec::new());
    if cont.is_empty() {
        return None;
    }
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
        let artist = i
            .get("playlistPanelVideoRenderer")
            .and_then(|v| v.get("shortBylineText"))
            .and_then(|v| v.get("runs"))
            .and_then(Value::as_array)
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"));
        let jso = json!({"id": id, "name": name, "artist": artist});
        arr.push(jso);
    }
    Some(arr)
}

pub async fn get_suggestions(query: &str) -> Result<Value, reqwest::Error> {
    let s = query
        .split(' ')
        .collect::<Vec<&str>>()
        .join("%20")
        .to_string();
    let q = concat_strings(Vec::from([SUGGESTION_SOURCE, "search?q=", s.as_str()]));
    let client = CLIENT.get().unwrap().clone();
    let mut body = None;
    if let Ok(b) = client
        .get(q)
        .header(USER_AGENT, AGENT)
        .header(REFERER, SUGGESTION_SOURCE)
        .send()
        .await
    {
        if let Ok(e) = b.json::<Value>().await {
            body = Some(e)
        }
    }

    if body.is_none() {
        return Ok(empty_json());
    }
    let body = body.unwrap();
    Ok(body)
}
