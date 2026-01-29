use serde_json::{Value, json};
use std::{env, fs::File, io::{Write, Result}};
use uuid::Uuid;
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

pub fn getsong_fmt(stream: &str, id: i32, audio_qual: &str) -> String {
    format!("{stream}id={id}&quality={audio_qual}")
}
pub fn home_format() -> String {
    format!(
        "{}/.local/share/mscply/cache.json",
        env::var("HOME").unwrap()
    )
}
pub fn mpd_url_builder(p1: &str, p2: &str, p3: &str) -> String {
    format!("{}{}{}", p1, p2, p3)
}
pub fn query_format(querybase: &str, something: String) -> String {
    format!("{querybase}{something}")
}
pub fn songpath_fmt(id: &str) -> String {
    format!(
        "{}/.local/share/mscply/songs/{}",
        env::var("HOME").unwrap(),
        id
    )
}
pub fn temp_format(uuid: Uuid) -> String {
    format!("{}/mpd_{uuid}", env::temp_dir().display())
}
pub fn playing(name: &str) -> String {
    format!("Playing {}", name)
}
pub fn queuelist_item(
    prefix: &str,
    title: &str,
    artist: &str,
    min: i64,
    sec: i64,
    qual: String,
) -> String {
    format!("{prefix}{title} — {artist} ({min}:{sec}, {qual})")
}
pub fn file_write(f: &mut File, data: &str) -> Result<()> {
    writeln!(f, "{}",data)?;
    Ok(())
}
pub fn status_print(status: &str, pause: &str, queue: &str) -> String {
    format!("Status: {}\nPaused: {}\nQueue: {}", status, pause, queue)
}
pub fn empty_json() -> Value {
    json!({})
}
pub fn ytmusic_search_fmt(smth: String) -> String {
    format!("https://music.youtube.com/search?q={}", smth)
}
pub fn songlink_fmt(id: &str, source: &str) -> String {
    format!("https://song.link/{source}/{id}")
}
pub fn ref_fmt(ytid: &str) -> String {
    format!(
        "https://music.youtube.com/watch?v={}&list=RDAMVM{}",
        ytid, ytid
    )
}
pub fn getqual_fmt(infostream: &str, id: &str) -> String {
    format!("https://{}/info/?id={}", infostream, id)
}
pub fn ytrecs_json(ytid: &str) -> Value {
    json!({
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
                "userAgent": "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0",
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
    })
}
pub fn query_json(name: &str) -> Value {
    json!({
        "context": {
            "client": {
                "hl": "en",
                "gl": "CA",
                "deviceMake": "",
                "deviceModel": "",
                "userAgent": "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0",
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
    })
}
