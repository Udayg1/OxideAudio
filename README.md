# 🎧 OxideAudio

**A high-performance music streaming application built in Rust.**

OxideAudio is designed for low-latency audio playback and efficient
streaming over networks. It leverages **mpv** for decoding and playback
while focusing on building a robust streaming, buffering, and control
layer in Rust.

------------------------------------------------------------------------

## 🚀 Features

-   ⚡ **Low-latency streaming**
-   🌐 **Network audio playback**
-   🔄 **Efficient buffering pipeline**
-   ⏩ **Smooth seeking**
-   🎚️ **mpv-powered decoding & playback**
-   🧵 **Concurrent streaming architecture**

------------------------------------------------------------------------

## 🛠️ Tech Stack

-   **Rust**
-   **libmpv / mpv** -- decoding & playback
-   **reqwest** -- networking
-   **tokio / threading** -- concurrency

------------------------------------------------------------------------

## 📦 Installation

``` bash
git clone https://github.com/Udayg1/oxideaudio.git
cd oxideaudio
cargo build --release
```

------------------------------------------------------------------------

## ▶️ Usage

``` bash
cargo run --release 
```

------------------------------------------------------------------------

## 🧠 Design Overview

OxideAudio focuses on **systems-level control around audio streaming**,
while delegating decoding to mpv:

-   Audio sources are fetched over the network
-   Streaming logic handles buffering and data flow
-   **MPV** is used as the backend for decoding and playback
-   The system is designed to handle:
    -   network interruptions
    -   seeking within streams
    -   continuous playback

This approach allows OxideAudio to combine **Rust's performance and
control** with **mpv's mature media capabilities**.

------------------------------------------------------------------------

## 📁 Project Structure

    src/
    ├── app/             # Entry point
    ├── player/          # mpv integration & playback control
    ├── network/         # Streaming & fetching logic
    ├── ui/              # TUI implementation
    └── macros/          # Heavy macro expansion

------------------------------------------------------------------------

## ⚙️ Goals

-   Build a **reliable streaming layer** in Rust
-   Integrate cleanly with **mpv as a backend**
-   Explore **concurrency and networking**
-   Maintain **low overhead and responsiveness**

------------------------------------------------------------------------

## 🧪 Future Improvements

-   [x] Playlist support\
-   [x] Local file playback\
-   [ ] Better error handling (network failures, retries)\
-   [x] CLI controls (pause, skip, seek)\
-   [ ] Improved buffering strategy\
-   [ ] GUI frontend

------------------------------------------------------------------------

## 🤝 Contributing

Contributions are welcome! Feel free to open issues or submit pull
requests.
