use base64::{Engine as _, engine::general_purpose};
use libmpv2::Mpv;
use reqwest::header::USER_AGENT;
use serde_json;
use serde_json::Value;
use std::env;
use std::io::{Write, stdin, stdout};

const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:145.0) Gecko/20100101 Firefox/145.0";
const QUERYBASE: &str = "https://maus.qqdl.site/search/?s=";
const STREAM: &str = "https://tidal.kinoplus.online/track/?";

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
    if idle {
        let _ = mpv.command("loadfile", &[url, "replace"]);
    } else {
        if init == 1 {
            let _ = mpv.command("loadfile", &[url, "append-play"]);
        } else {
            let _ = mpv.command("loadfile", &[url, "append"]);
        }
    }
}

async fn get_similar(id: i32, token: &str) -> Result<Value, reqwest::Error> {
    let url = format!(
        "https://openapi.tidal.com/v2/tracks/{}/relationships/similarTracks?countryCode=US&include=similarTracks",
        id
    );
    let client = reqwest::Client::new();
    let response: Value = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?
        .json()
        .await?;
    Ok(response)
}

async fn get_token() -> Result<Value, reqwest::Error> {
    let client = reqwest::Client::new();

    let res = client
            .post("https://auth.tidal.com/v1/oauth2/token")
            .header(
                "Authorization",
                "Basic Rkhlc3g3bnZudlVsQVVqeTpuTzVOZUYxVEV0WW04cDBPbnU2NnZwMGxVY2p2Qk5pOVZJTlR0WTR2ZjBnPQ==",
            )
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await?.json().await?;
    Ok(res)
}

async fn queue_similar(json: &[Value], mpv: &mut Mpv) {
    for item in json.iter().take(7) {
        let iid = item.get("id").and_then(|v| v.as_str()).unwrap();
        let id: i32 = iid.parse().unwrap();
        let mut audio_quality = "LOSSLESS";
        let qualities: Vec<&str> = item
            .get("attributes")
            .and_then(|attr| attr.get("mediaTags"))
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|tag| tag.as_str())
            .collect();
        let qual = "HIRES_LOSSLESS";
        if qualities.iter().any(|v| v as &str == qual) {
            audio_quality = "HI_RES_LOSSLESS";
        }
        let res = get_song(id, audio_quality).await.unwrap();
        let song = decode_base64(
            res.get("data")
                .and_then(|d| d.get("manifest"))
                .and_then(|v| v.as_str())
                .unwrap(),
        );
        if song.starts_with("<xml") {
            queue_mpd_song(mpv, &song, 0);
        } else {
            if let Ok(json) = serde_json::from_str::<Value>(&song) {
                if let Some(urls) = json.get("urls").and_then(|v| v.as_array()) {
                    if let Some(first_url) = urls.first().and_then(Value::as_str) {
                        queue_song(mpv, first_url, 0);
                    } else {
                        eprintln!("'urls' array is empty or first element is not a string");
                    }
                } else {
                    eprintln!("No 'urls' array found");
                }
            }
        }
    }
}

async fn add_song(mpv: &mut Mpv, token: &str) {
    print!("Enter song name (or q to quit): ");
    stdout().flush().unwrap();
    let mut name = String::new();
    stdin().read_line(&mut name).unwrap();
    let name = name.trim();
    if name.eq_ignore_ascii_case("q") {
        return;
    }
    let data = search_result(name).await.unwrap();
    println!("\nResults: ");
    let items = data
        .get("data")
        .and_then(|d| d.get("items"))
        .and_then(|arr| arr.as_array());
    if let Some(items) = items {
        let items = &items[..items.len().min(5)];
        for (i, track) in items.iter().enumerate() {
            let title = track
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Unknown Title");
            let artist = track
                .get("artist")
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown Artist");
            println!("{}. {} - {}", i + 1, title, artist);
        }

        print!("\nSelect track number: ");
        stdout().flush().unwrap();
        let mut input = String::new();
        stdin().read_line(&mut input).unwrap();
        let choice: usize = input.trim().parse().unwrap_or(0);
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
            let res = match get_similar(id, token).await {
                Ok(v) => v,
                Err(_e) => return,
            };
            let included = match res.get("included").and_then(|v| v.as_array()) {
                Some(v) => v,
                None => return,
            };
            queue_similar(included, mpv).await;
        }
    }
}

#[tokio::main]
async fn main() {
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
    let resp = get_token().await.unwrap();
    let token = resp.get("access_token").and_then(Value::as_str).unwrap();
    loop {
        print!("Options: (p)ause, (r)esume, (h)alt, (s)kip, (v#)olume, (a)dd a song -> ");
        stdout().flush().unwrap();
        let mut s = String::new();
        stdin().read_line(&mut s).expect("Failed to read line");
        let name = s.trim().to_lowercase();
        if name.eq_ignore_ascii_case("h") {
            break;
        } else if name.eq_ignore_ascii_case("p") {
            mpv.set_property("pause", true).unwrap();
        } else if name.eq_ignore_ascii_case("r") {
            mpv.set_property("pause", false).unwrap();
        } else if name.eq_ignore_ascii_case("s") {
            mpv.command("playlist-next", &["force"]).unwrap();
        } else if name.eq_ignore_ascii_case("a") {
            add_song(&mut mpv, token).await;
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
}
