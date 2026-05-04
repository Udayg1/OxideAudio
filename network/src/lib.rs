use macros::*;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, REFERER, USER_AGENT};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{env, fs};
use uuid;

pub static PREF_QUAL: OnceLock<String> = OnceLock::new();
pub static INFOSTREAM: OnceLock<bool> = OnceLock::new();
static CLIENT: OnceLock<Client> = OnceLock::new();
pub static IS_CACHING: AtomicBool = AtomicBool::new(false);
pub const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:149.0) Gecko/20100101 Firefox/149.0";
pub static API: &str = "https://t2tunes.site/api/amazon-music";
pub struct CacheItem {
    pub path: String,
    pub index: usize,
}

// pub async fn get_image(path_str: &str) -> String {
//     let full_path = concat_strings(Vec::from([
//         "https://resources.tidal.com/images/",
//         &path_str.split("-").collect::<Vec<&str>>().join("/"),
//         "/640x640.jpg",
//     ]));
//     let client = Client::new();
//     let res = client
//         .get(full_path)
//         .header(USER_AGENT, AGENT)
//         .send()
//         .await
//         .unwrap()
//         .bytes()
//         .await
//         .unwrap();
//     let mut path = String::new();
//     path += env::temp_dir().to_str().unwrap();
//     path += "/";
//     path += &uuid::Uuid::new_v4().to_string();
//     path += ".jpg";
//     fs::write(&path, res).unwrap();
//     path
// }

pub async fn get_quality(id: &str) -> Value {
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
            return json!({"quality": ""});
        }
    };
    if !response.is_err() {
        resp = Some(response.unwrap().json::<Value>().await.expect("JSON ERROR"));
    }
    if resp.is_none() {
        return json!({"quality": ""});
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
        let res = cli
            .get("https://t2tunes.site/api/status")
            .send()
            .await
            .unwrap();
        let jsn = res.json::<Value>().await.unwrap();
        if jsn.get("amazonMusic").and_then(Value::as_str) == Some("up") {
            INFOSTREAM.set(true).unwrap();
        } else {
            INFOSTREAM.set(false).unwrap();
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
        if let Ok(jsn) = b.json::<Value>().await {
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

pub async fn search_result(query: &str) -> Result<Value, reqwest::Error> {
    let s = query
        .split(' ')
        .collect::<Vec<&str>>()
        .join("+")
        .to_string();
    let q = concat_strings(Vec::from([
        API,
        "/search?query=",
        s.as_str(),
        "&types=track",
    ]));
    let client = CLIENT.get().unwrap().clone();
    let mut body = None;
    if let Ok(b) = client
        .get(q)
        .header(USER_AGENT, AGENT)
        .header(REFERER, API)
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

pub async fn get_songlink_data(id: &str, source: &str) -> Value {
    let url = concat_strings(Vec::from(["https://song.link/", source, "/", id]));
    let client = CLIENT.get().unwrap().clone();
    let mut response = client.get(&url).header(USER_AGENT, AGENT).send().await;
    while response.is_err() {
        std::thread::sleep(Duration::from_secs(10));
        response = client.get(&url).header(USER_AGENT, AGENT).send().await;
    }
    let re =
        regex::Regex::new(r#"<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
            .unwrap();
    let json_text = re
        .captures(&response.unwrap().text().await.unwrap())
        .unwrap()[1]
        .to_string();
    serde_json::from_str(&json_text).unwrap()
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

pub fn get_ytrec_array(recs: Value) -> Vec<Value> {
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
    arr
}

pub fn extract_amazon_id(json: &Value) -> Option<String> {
    let sections = json
        .get("props")?
        .get("pageProps")?
        .get("pageData")?
        .get("sections")?
        .as_array()?;
    for section in sections {
        if let Some(links) = section.get("links").and_then(|l| l.as_array()) {
            for link in links {
                if link.get("platform")?.as_str()? == "amazonStore" {
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
    None
}
