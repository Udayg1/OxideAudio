use macros::*;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, REFERER, USER_AGENT};
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::io::Write;
use std::sync::OnceLock;
use std::time::Duration;
use std::{env, fs};
use uuid;

pub static PREF_QUAL: OnceLock<String> = OnceLock::new();
pub static QUERYBASE: OnceLock<String> = OnceLock::new();
pub static STREAM: OnceLock<String> = OnceLock::new();
pub static INFOSTREAM: OnceLock<String> = OnceLock::new();
pub const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";

pub async fn get_quality(id: &str) -> String {
    let cli = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let res = cli
        .get(concat_strings(Vec::from([
            INFOSTREAM.get().unwrap(),
            "/info/?id=",
            id,
        ])))
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
    let pref = PREF_QUAL.get().unwrap();
    if !qual.is_none() {
        let mut quality = qual.unwrap();
        if quality == "LOSSLESS" && (pref == "HIGH" || pref == "LOW") {
            return pref.to_string();
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
        quality.to_string()
    } else {
        "".to_string()
    }
}

pub async fn cache_next_song(url: &str) -> String {
    let cli = Client::new();
    let path = concat_strings(Vec::from([
        env::temp_dir().to_str().unwrap(),
        "/",
        &uuid::Uuid::new_v4().to_string(),
    ]));
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
                return "".to_string();
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
        path
    } else {
        // if fs::metadata(&path).is_ok() {
        //     return path;
        // }
        // eprintln!("{}/ -- {url}", env::temp_dir().display());
        let bytes = cli.get(url).send().await.unwrap().bytes().await;
        fs::write(&path, bytes.unwrap()).ok();
        path
    }
}

pub fn set_url() {
    tokio::spawn(async {
        let js_url = "https://tidal.squid.wtf/_app/immutable/chunks/DuHawVqQ.js";
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .user_agent(AGENT)
            .build()
            .unwrap();
        let mut res = client
            .get(js_url)
            .header(REFERER, "https://tidal.squid.wtf")
            .send()
            .await;
        let mut count = 0;
        while res.is_err() && count <= 5 {
            res = client
                .get(js_url)
                .header(REFERER, "https://tidal.squid.wtf")
                .send()
                .await;
            count += 1;
        }
        let js_obj = res.unwrap().text().await.unwrap();
        let sind = js_obj.find("J=").unwrap() + 2;
        let eind = &js_obj[sind + 1..].find("}]").unwrap() + 3 + sind;
        let arr = &js_obj[sind..eind];
        let mut s = arr.to_string();
        s = s.replace("!1", "false");
        s = s.replace("!0", "true");

        for key in ["name", "baseUrl", "weight", "requiresProxy", "category"] {
            s = s.replace(&format!("{key}:"), &format!("\"{key}\":"));
        }
        let mut json_arr: Vec<Value> = serde_json::from_str(&s).unwrap();
        json_arr.sort_by_key(|x| Reverse(x.get("weight").and_then(Value::as_i64)));
        INFOSTREAM
            .set(
                json_arr[0]
                    .get("baseUrl")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
            )
            .unwrap();
        STREAM
            .set(concat_strings(Vec::from([
                json_arr[0].get("baseUrl").and_then(Value::as_str).unwrap(),
                "/track/?",
            ])))
            .unwrap();
        QUERYBASE
            .set(concat_strings(Vec::from([
                json_arr[0].get("baseUrl").and_then(Value::as_str).unwrap(),
                "/search/?s=",
            ])))
            .unwrap();
    });
}

pub async fn get_song(id: i32, audio_quality: &str) -> Result<Value, reqwest::Error> {
    let fin_url = concat_strings(Vec::from([
        STREAM.get().unwrap(),
        "id=",
        &id.to_string(),
        "&quality=",
        audio_quality,
    ]));
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let body: Value = client
        .get(fin_url)
        .header(USER_AGENT, AGENT)
        .send()
        .await?
        .json()
        .await?;
    Ok(body)
}

pub async fn search_result(query: &str) -> Result<Value, reqwest::Error> {
    let s: Vec<&str> = query.split(' ').collect();
    let q = concat_strings(Vec::from([
        QUERYBASE.get().unwrap(),
        s.join("%20").as_str(),
    ]));
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let body: Value = client
        .get(q)
        .header(USER_AGENT, AGENT)
        .send()
        .await?
        .json()
        .await?;
    Ok(body)
}

pub async fn get_songlink_data(id: &str, source: &str) -> Value {
    let url = concat_strings(Vec::from(["https://song.link/", source, "/", id]));
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap();
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
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

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
    first
}

pub async fn get_ytrecs(ytid: &str) -> Value {
    if ytid.is_empty() {
        return empty_json();
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
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
    let client = Client::new();
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
