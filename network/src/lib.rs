use macros::*;
use rand::{rng, seq::SliceRandom};
use regex::Regex;
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
pub const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";
pub static API: OnceLock<Vec<Value>> = OnceLock::new();
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

fn return_shuffled() -> Vec<Value> {
    let mut v = API.get().unwrap().clone();
    v.shuffle(&mut rng());
    v
}

pub async fn get_quality(id: &str) -> Value {
    // let url = ;
    let cli = CLIENT.get().unwrap().clone();
    let v = return_shuffled();
    let mut resp = None;
    for i in v {
        let response = cli
            .get(concat_strings(Vec::from([
                i.get("url").and_then(Value::as_str).unwrap(),
                "/info/?id=",
                id,
            ])))
            .header(USER_AGENT, AGENT)
            .timeout(Duration::from_secs(7))
            .send()
            .await
            .unwrap()
            .error_for_status();
        if !response.is_err() {
            resp = Some(response.unwrap().json::<Value>().await.expect("JSON ERROR"));
            break;
        }
    }
    let res = resp.expect("Status error");
    let qual = res
        .get("data")
        .and_then(|v| v.get("audioQuality"))
        .and_then(Value::as_str);
    let pref = PREF_QUAL.get().unwrap();
    if !qual.is_none() {
        let mut quality = qual.unwrap();
        if quality == "LOSSLESS" && (pref == "HIGH" || pref == "LOW") {
            return json!({"quality":pref.to_string(), "duration": res.get("data").and_then(|v| v.get("duration")).and_then(Value::as_i64).unwrap(),"image": res.get("data").and_then(|v| v.get("album")).and_then(|v| v.get("cover")).and_then(Value::as_str).unwrap()});
        }
        let tags = res
            .get("data")
            .and_then(|v| v.get("mediaMetadata"))
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_array)
            .unwrap();
        if tags.iter().any(|v| v.as_str() == Some("HIRES_LOSSLESS")) {
            quality = "HI_RES_LOSSLESS";
        }
        json!({"quality":quality.to_string(), "duration": res.get("data").and_then(|v| v.get("duration")).and_then(Value::as_i64).unwrap(), "image": res.get("data").and_then(|v| v.get("album")).and_then(|v| v.get("cover")).and_then(Value::as_str).unwrap()})
    } else {
        json!({"quality": ""})
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
        let res = cli.get("https://monochrome.tf").send().await.unwrap();
        let text = res.text().await.unwrap();
        let re = regex::Regex::new(r"assets/index-[^/]+\.js").unwrap();
        let mut indexjs = "";
        for m in re.find_iter(&text) {
            indexjs = m.as_str();
            break;
        }
        let res = cli
            .get(concat_strings(Vec::from([
                "https://monochrome.tf/",
                indexjs,
            ])))
            .send()
            .await
            .unwrap();
        let text = res.text().await.unwrap();
        let re = Regex::new(r#"INSTANCES_URLS:*\[([^\]]+)\]"#).unwrap();
        let mut arr = Vec::new();
        if let Some(caps) = re.captures(&text) {
            let inner = &caps[1];
            let url_re = Regex::new(r#""([^"]+)""#).unwrap();

            arr = url_re
                .captures_iter(inner)
                .map(|c| c.get(1).unwrap().as_str().to_string())
                .collect::<Vec<String>>();
        }
        let mut url_json = Vec::new();
        for i in arr {
            let res = cli.get(&i).send().await.unwrap().error_for_status();
            if !res.is_err() {
                let text = res.unwrap().text().await.unwrap();
                let js = serde_json::from_str::<Value>(&text).unwrap();
                let jss = js.get("streaming").and_then(Value::as_array).unwrap();
                url_json.append(&mut jss.clone());
                break;
            }
        }
        CLIENT.set(cli.clone()).unwrap();
        INFOSTREAM.set(true).unwrap();
        API.set(url_json.clone()).unwrap();
        for i in &url_json {
            match cli
                .head(i.get("url").and_then(Value::as_str).unwrap().to_string())
                .send()
                .await
            {
                _ => {}
            }
        }
    });
}

pub async fn get_song(id: i32, audio_quality: &str) -> Result<Value, reqwest::Error> {
    let fin_url = concat_strings(Vec::from([
        "/track/?id=",
        &id.to_string(),
        "&quality=",
        audio_quality,
    ]));
    let client = CLIENT.get().unwrap().clone();
    let v = return_shuffled();
    let mut body = None;
    for i in &v {
        if let Ok(b) = client
            .get(concat_strings(Vec::from([
                i.get("url").and_then(Value::as_str).unwrap(),
                &fin_url,
            ])))
            .timeout(Duration::from_secs(5))
            .header(USER_AGENT, AGENT)
            .send()
            .await
        {
            let b = b.error_for_status();
            if !b.is_err() {
                body = Some(b);
                break;
            }
        }
    }
    let body = body.unwrap();
    Ok(body?.json().await?)
}

pub async fn search_result(query: &str) -> Result<Value, reqwest::Error> {
    let s = query
        .split(' ')
        .collect::<Vec<&str>>()
        .join("%20")
        .to_string();
    let q = concat_strings(Vec::from(["/search/?s=", s.as_str()]));
    let client = CLIENT.get().unwrap().clone();
    let v = return_shuffled();
    let mut body = None;
    for i in &v {
        if let Ok(b) = client
            .get(concat_strings(Vec::from([
                i.get("url").and_then(Value::as_str).unwrap(),
                &q,
            ])))
            .timeout(Duration::from_secs(7))
            .header(USER_AGENT, AGENT)
            .send()
            .await
        {
            let b = b.error_for_status();
            if !b.is_err() {
                body = Some(b);
                break;
            }
        }
    }
    let body = body.unwrap();
    Ok(body?.json().await?)
}

pub async fn get_songlink_data(id: &str, source: &str) -> Value {
    let url = concat_strings(Vec::from(["https://song.link/", source, "/", id]));
    let client = CLIENT.get().unwrap().clone();
    let response = client.get(&url).header(USER_AGENT, AGENT).send().await;
    if response.is_err() {
        return empty_json();
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
    let mut resp = client
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
        .timeout(Duration::from_secs(7))
        .json(&body)
        .send()
        .await;
    while resp.is_err() {
        resp = client
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
    }
    let res = resp.unwrap().text().await.unwrap();
    serde_json::from_str(&res).unwrap()
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

pub async fn cache_mpd_song(mpd_string: &str, tidal_id: &str) {
    let new: Vec<&str> = mpd_string.split(" ").collect();
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
    let client = CLIENT.get().unwrap().clone();
    init = init.replace("amp;", "");
    let new_init = init.split("$Number$").collect::<Vec<&str>>();
    let r = r.parse::<u32>().unwrap();
    let mut bytes: Vec<u8> = Vec::new();
    for i in 0..=r + 2 {
        let resp = client
            .get(concat_strings(Vec::from([
                new_init[0],
                &i.to_string(),
                new_init[1],
            ])))
            .send()
            .await;
        if resp.is_err() {
            return;
        }
        let chunk = resp.unwrap().bytes().await.unwrap();
        bytes.extend_from_slice(&chunk);
    }
    let mut handle = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(concat_strings(Vec::from([
            &env::var("HOME").unwrap(),
            "/.local/share/mscply/songs/",
            tidal_id,
        ])))
        .unwrap();
    handle.write_all(&bytes).unwrap();
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

pub fn extract_tidal_id(json: &Value) -> Option<String> {
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
    None
}
