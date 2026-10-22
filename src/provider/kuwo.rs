use crate::{
    lyrics,
    model::{clean, normalize_id, Error, Result, Track},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::read::ZlibDecoder;
use md5::{Digest, Md5};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const UA: &str = "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.0; .NET CLR 1.1.4322)";
const VERSION: &str = "MUSIC_8.7.4.0_BDS3";
const CLIENT: &str = "58365624";
pub use super::traits::Stream;
#[derive(Clone, Debug)]
struct Quality {
    name: String,
    s1: u64,
    s2: u64,
    size: u64,
    extension: String,
}

fn xor(data: &mut [u8], key: &[u8]) {
    for (i, byte) in data.iter_mut().enumerate() {
        *byte ^= key[i % key.len()];
    }
}
pub fn encode_query(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    if data.is_empty() {
        return data;
    }
    let len = data.len();
    data.rotate_right(0x85 % len);
    xor(&mut data, b"KoOtOiTvINGwd");
    data.rotate_left(0x47 % len);
    xor(&mut data, b"_Y8g2E6n0E1i7L5t2IoOoNk");
    STANDARD.encode(data).into_bytes()
}
pub fn decode_reply(raw: &[u8]) -> Result<Vec<u8>> {
    let mut data = STANDARD.decode(raw).unwrap_or_else(|_| raw.to_vec());
    if data.is_empty() {
        return Err(Error::new("音源返回空响应"));
    }
    let len = data.len();
    xor(&mut data, b"_Y8g2E6n0E1i7L5t2IoOoNk");
    data.rotate_right(0x47 % len);
    xor(&mut data, b"KoOtOiTvINGwd");
    data.rotate_left(0x85 % len);
    let mut uncompressed = vec![];
    if ZlibDecoder::new(data.as_slice())
        .take(4 * 1024 * 1024)
        .read_to_end(&mut uncompressed)
        .is_ok()
    {
        Ok(uncompressed)
    } else {
        Ok(data)
    }
}
pub fn parse_json(bytes: &[u8]) -> Result<Value> {
    if let Ok(value) = serde_json::from_slice(bytes) {
        return Ok(value);
    }
    // Legacy search replies use Python-style single-quoted dictionaries.
    // Convert quotes without changing apostrophes inside double-quoted strings.
    let raw = String::from_utf8_lossy(bytes);
    let mut output = String::new();
    let mut quote = None;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    if next == '\'' && q == '\'' {
                        output.push('\'');
                    } else {
                        output.push('\\');
                        output.push(next);
                    }
                }
            } else if c == q {
                output.push('"');
                quote = None;
            } else if c == '"' {
                output.push_str("\\\"");
            } else {
                output.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            output.push('"');
        } else if c.is_ascii_alphabetic() {
            let mut token = String::from(c);
            while chars.peek().is_some_and(|v| v.is_ascii_alphabetic()) {
                token.push(chars.next().unwrap());
            }
            output.push_str(match token.as_str() {
                "None" => "null",
                "True" => "true",
                "False" => "false",
                _ => &token,
            });
        } else {
            output.push(c);
        }
    }
    serde_json::from_str(&output).map_err(|_| Error::new("音源响应格式无法解析"))
}
fn text(value: &Value) -> String {
    match value {
        Value::String(v) => v.clone(),
        Value::Number(v) => v.to_string(),
        _ => String::new(),
    }
}
fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
        .unwrap_or(0.)
}
fn encode(value: &str) -> String {
    let mut output = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            output.push(b as char);
        } else {
            output.push_str(&format!("%{b:02X}"));
        }
    }
    output
}
fn url(base: &str, pairs: &[(&str, String)]) -> String {
    format!(
        "{base}?{}",
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    )
}
pub(crate) fn range(header: &str) -> Option<(u64, u64, u64)> {
    let (span, total) = header.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}
pub fn valid_audio(head: &[u8]) -> bool {
    [b"ID3".as_slice(), b"fLaC", b"OggS", b"RIFF"]
        .iter()
        .any(|v| head.starts_with(v))
        || (head.len() >= 2 && head[0] == 255 && head[1] & 0xE0 == 0xE0)
}

#[derive(Clone)]
pub struct KuwoProvider {
    agent: ureq::Agent,
}
pub type Provider = KuwoProvider;
impl KuwoProvider {
    pub fn new(direct: bool) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .try_proxy_from_env(!direct)
                .build(),
        }
    }
    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }
    fn request(
        &self,
        address: &str,
        data: Option<&[u8]>,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<ureq::Response> {
        if !address.starts_with("http://") && !address.starts_with("https://") {
            return Err(Error::new("无效的媒体地址"));
        }
        let mut request = self
            .agent
            .request(if data.is_some() { "POST" } else { "GET" }, address)
            .set("User-Agent", UA)
            .timeout(timeout);
        for (key, value) in headers {
            request = request.set(key, value);
        }
        let response = if let Some(data) = data {
            request.send_bytes(data)
        } else {
            request.call()
        };
        response.map_err(|error| match error {
            ureq::Error::Status(status, _) => {
                Error::coded("network", format!("音源请求失败：HTTP {status}"))
            }
            _ => Error::coded("network", "音源请求超时或网络不可用"),
        })
    }
    fn read(
        &self,
        address: &str,
        data: Option<&[u8]>,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let mut bytes = vec![];
        self.request(address, data, headers, timeout)?
            .into_reader()
            .take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::coded("network", "音源响应读取失败"))?;
        Ok(bytes)
    }
    fn json(&self, address: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Value> {
        parse_json(&self.read(address, None, headers, timeout)?)
    }
    fn track(item: &Value) -> Result<Track> {
        let id = normalize_id(&text(
            item.get("MUSICRID")
                .or_else(|| item.get("RID"))
                .unwrap_or(&Value::Null),
        ))?;
        Ok(Track {
            id,
            title: clean(&text(
                item.get("SONGNAME")
                    .or_else(|| item.get("NAME"))
                    .unwrap_or(&Value::Null),
            )),
            artist: clean(&text(&item["ARTIST"])),
            album: clean(&text(&item["ALBUM"])),
            duration: number(&item["DURATION"]),
            formats: text(&item["FORMATS"]),
            path: String::new(),
        })
    }
    pub fn search(&self, keyword: &str, page: usize, limit: usize) -> Result<(usize, Vec<Track>)> {
        let address = url(
            "https://search.kuwo.cn/r.s",
            &[
                ("all", keyword.into()),
                ("ft", "music".into()),
                ("newsearch", "1".into()),
                ("alflac", "1".into()),
                ("itemset", "web_2013".into()),
                ("client", "kt".into()),
                ("cluster", "0".into()),
                ("pn", page.to_string()),
                ("rn", limit.to_string()),
                ("vermerge", "1".into()),
                ("rformat", "json".into()),
                ("encoding", "utf8".into()),
                ("show_copyright_off", "1".into()),
                ("pcmp4", "1".into()),
                ("vipver", VERSION.into()),
                ("plat", "pc".into()),
                ("devid", CLIENT.into()),
            ],
        );
        let result = self.json(&address, &[], Duration::from_secs(12))?;
        let tracks = result["abslist"]
            .as_array()
            .ok_or_else(|| Error::new("搜索接口未返回歌曲列表"))?
            .iter()
            .map(Self::track)
            .collect::<Result<_>>()?;
        Ok((number(&result["TOTAL"]) as usize, tracks))
    }
    pub fn details(&self, id: &str) -> Result<Track> {
        let id = normalize_id(id)?;
        let rid = id.trim_start_matches("kuwo:");
        let result = self.json(
            &url(
                "https://search.kuwo.cn/r.s",
                &[
                    ("type", "music".into()),
                    ("rformat", "json".into()),
                    ("encoding", "utf8".into()),
                    ("rid", format!("MUSIC_{rid}")),
                ],
            ),
            &[],
            Duration::from_secs(10),
        )?;
        let items = result
            .as_array()
            .or_else(|| result["abslist"].as_array())
            .ok_or_else(|| Error::new("找不到歌曲信息"))?;
        for item in items {
            let Some(object) = item.as_object() else {
                continue;
            };
            let mut mapped = serde_json::Map::new();
            for (key, value) in object {
                mapped.insert(key.to_uppercase(), value.clone());
            }
            mapped
                .entry("MUSICRID")
                .or_insert(json!(format!("MUSIC_{rid}")));
            if let Ok(track) = Self::track(&Value::Object(mapped)) {
                if track.id == id {
                    return Ok(track);
                }
            }
        }
        Err(Error::new("无法获取歌曲信息，请先搜索"))
    }
    fn qualities(&self, track: &Track) -> Result<Vec<Quality>> {
        let rid = normalize_id(&track.id)?;
        let bytes = self.read(&format!("http://search.kuwo.cn/r.s?stype=musicinfo&itemset=music_2014&alflac=1&pcmp4=1&ids=MUSIC_{}",rid.trim_start_matches("kuwo:")),None,&[],Duration::from_secs(8))?;
        if bytes.len() < 8 {
            return Err(Error::new("音质响应不完整"));
        }
        let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let compressed = bytes
            .get(8..8 + length)
            .ok_or_else(|| Error::new("音质响应长度错误"))?;
        let mut raw = String::new();
        ZlibDecoder::new(compressed)
            .take(4 * 1024 * 1024)
            .read_to_string(&mut raw)?;
        let mut qualities = vec![];
        for (name, key, extension) in [
            ("flac", "ALFLAC", ".flac"),
            ("320k", "MP3H", ".mp3"),
            ("128k", "MP3128", ".mp3"),
        ] {
            let Some(line) = raw
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{key}=")))
            else {
                continue;
            };
            let fields: Vec<u64> = line
                .split('|')
                .take(3)
                .filter_map(|v| v.split_once(':')?.1.parse().ok())
                .collect();
            if fields.len() == 3 && fields[2] > 0 {
                qualities.push(Quality {
                    name: name.into(),
                    s1: fields[0],
                    s2: fields[1],
                    size: fields[2],
                    extension: extension.into(),
                });
            }
        }
        Ok(qualities)
    }
    fn resolve(&self, q: &Quality) -> Result<(String, u64)> {
        let query = format!("<001><U_QRY>|<{},{}>|<{CLIENT}><{VERSION}><kwmusic_web_1_bds_20171222.exe>|<>|<rid>|<uip:0.0.0.0>|<new>|<nat:3>|<flags:262144><speer>|<ipdeny:no>|<loginid:>",q.s1,q.s2);
        let data = self.read(
            "http://deliver.kuwo.cn/yl_res_manage.search",
            Some(&encode_query(&query)),
            &[("Connection", "close")],
            Duration::from_secs(6),
        )?;
        let bytes = decode_reply(&data)?;
        let raw = String::from_utf8_lossy(&bytes);
        let extract = |marker: &str| {
            raw.split_once(marker)
                .and_then(|(_, v)| v.split('>').next())
        };
        let address = extract("<URL><1>|<")
            .ok_or_else(|| Error::coded("source_unavailable", "该档位未提供播放地址"))?;
        Ok((
            address.into(),
            extract("<FILE_LEN><")
                .and_then(|v| v.parse().ok())
                .unwrap_or(q.size),
        ))
    }
    fn probe(&self, address: &str, expected: u64) -> Result<u64> {
        let response = self.request(
            address,
            None,
            &[("Range", "bytes=0-4095")],
            Duration::from_secs(6),
        )?;
        let size = response
            .header("Content-Range")
            .and_then(range)
            .map(|(_, _, n)| n)
            .or_else(|| {
                response
                    .header("Content-Length")
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(0);
        let mut head = vec![];
        response.into_reader().take(4096).read_to_end(&mut head)?;
        if !valid_audio(&head) {
            return Err(Error::new("返回内容不是可识别的音频"));
        }
        if expected > 0 && size > 0 && size < expected * 8 / 10 {
            return Err(Error::coded(
                "incomplete_audio",
                "上游仅返回试听或不完整音频",
            ));
        }
        Ok(size)
    }
    pub fn stream(
        &self,
        track: &Track,
        preference: &str,
        cancelled: &AtomicBool,
    ) -> Result<Stream> {
        let mut qualities = self.qualities(track)?;
        if preference == "320k" {
            qualities.retain(|q| q.name != "flac");
        }
        if preference == "128k" {
            qualities.retain(|q| q.name == "128k");
        }
        let mut errors = vec![];
        let mut details = vec![];
        let start = Instant::now();
        for public in [false, true] {
            for q in &qualities {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(Error::coded("cancelled", "已取消"));
                }
                if start.elapsed() > Duration::from_secs(44) {
                    return Err(Error::coded("network", "音源解析超时"));
                }
                if public && q.extension != ".mp3" {
                    continue;
                }
                let result = (|| {
                    let (address, size) = if public {
                        let result = self.json(
                            &url(
                                "https://antiserver.kuwo.cn/anti.s",
                                &[
                                    ("type", "convert_url3".into()),
                                    ("rid", track.id.trim_start_matches("kuwo:").into()),
                                    ("format", "mp3".into()),
                                    ("br", q.name.clone()),
                                    ("response", "url".into()),
                                ],
                            ),
                            &[],
                            Duration::from_secs(6),
                        )?;
                        let address = result["url"]
                            .as_str()
                            .filter(|v| !v.is_empty())
                            .ok_or_else(|| {
                                Error::coded("source_unavailable", "该档位未提供播放地址")
                            })?;
                        (address.to_string(), q.size)
                    } else {
                        self.resolve(q)?
                    };
                    let actual = self.probe(&address, q.size)?;
                    Ok(Stream {
                        url: address,
                        quality: q.name.clone(),
                        size: if actual > 0 { actual } else { size },
                        extension: q.extension.clone(),
                    })
                })();
                match result {
                    Ok(stream) => return Ok(stream),
                    Err(error) => {
                        details.push(format!(
                            "{} {}: {}",
                            if public { "公开接口" } else { "P2P" },
                            q.name,
                            error
                        ));
                        errors.push(error);
                    }
                }
            }
        }
        let incomplete = errors.iter().any(|e: &Error| e.code == "incomplete_audio");
        let network = !errors.is_empty() && errors.iter().all(|e| e.code == "network");
        Err(Error {
            message: if incomplete {
                "上游仅返回试听或不完整音频"
            } else if network {
                "音源网络请求失败"
            } else {
                "当前音源未提供完整播放地址"
            }
            .into(),
            code: if incomplete {
                "incomplete_audio"
            } else if network {
                "network"
            } else {
                "source_unavailable"
            }
            .into(),
            details,
        })
    }
    pub fn lyrics(&self, track: &Track) -> Result<String> {
        if !track.path.is_empty() {
            return Ok(
                fs::read_to_string(Path::new(&track.path).with_extension("lrc"))
                    .unwrap_or_default(),
            );
        }
        let start = Instant::now();
        let query = |address: &str, headers: &[(&str, &str)]| -> Result<Value> {
            let remaining = Duration::from_secs(20)
                .checked_sub(start.elapsed())
                .ok_or_else(|| Error::new("歌词获取超时"))?;
            self.json(address, headers, remaining.min(Duration::from_secs(4)))
        };
        for scheme in ["https", "http"] {
            if let Ok(data) = query(
                &format!(
                    "{scheme}://m.kuwo.cn/newh5/singles/songinfoandlrc?musicId={}&httpsStatus=1",
                    track.id.trim_start_matches("kuwo:")
                ),
                &[
                    ("Referer", "https://m.kuwo.cn/h5app/"),
                    (
                        "User-Agent",
                        "Mozilla/5.0 (Linux; Android 12) AppleWebKit/537.36 Mobile Safari/537.36",
                    ),
                ],
            ) {
                let mut output = String::new();
                for line in data["data"]["lrclist"].as_array().unwrap_or(&vec![]) {
                    let time = number(&line["time"]);
                    if !time.is_finite() || time < 0. {
                        continue;
                    }
                    if let Some(words) = line["lineLyric"].as_str() {
                        let centis = (time * 100.).round() as u64;
                        output.push_str(&format!(
                            "[{:02}:{:02}.{:02}]{}\n",
                            centis / 6000,
                            centis / 100 % 60,
                            centis % 100,
                            clean(words)
                        ));
                    }
                }
                if !lyrics::parse(&output).is_empty() {
                    return Ok(output);
                }
            }
        }
        let title = lyrics::title(&track.title);
        for keyword in [format!("{} {title}", track.artist), title] {
            if let Ok(result) = query(
                &url(
                    "https://krcs.kugou.com/search",
                    &[
                        ("ver", "1".into()),
                        ("man", "yes".into()),
                        ("client", "mobi".into()),
                        ("keyword", keyword.clone()),
                        ("duration", (track.duration * 1000.).round().to_string()),
                        ("hash", "".into()),
                    ],
                ),
                &[],
            ) {
                for candidate in result["candidates"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .take(10)
                {
                    let artists = text(&candidate["singer"])
                        .split(['&', '/', '、', ',', '，'])
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    if !lyrics::matches(
                        track,
                        &text(&candidate["song"]),
                        &artists,
                        number(&candidate["duration"]) / 1000.,
                    ) {
                        continue;
                    }
                    if let Ok(result) = query(
                        &url(
                            "https://lyrics.kugou.com/download",
                            &[
                                ("ver", "1".into()),
                                ("client", "pc".into()),
                                ("id", text(&candidate["id"])),
                                ("accesskey", text(&candidate["accesskey"])),
                                ("fmt", "lrc".into()),
                                ("charset", "utf8".into()),
                            ],
                        ),
                        &[],
                    ) {
                        if let Ok(bytes) = STANDARD.decode(text(&result["content"])) {
                            let raw = String::from_utf8_lossy(&bytes).into_owned();
                            if !lyrics::parse(&raw).is_empty() {
                                return Ok(raw);
                            }
                        }
                    }
                }
            }
            if let Ok(result) = query(
                &url(
                    "https://music.163.com/api/search/get/web",
                    &[("type", "1".into()), ("limit", "10".into()), ("s", keyword)],
                ),
                &[],
            ) {
                for song in result["result"]["songs"].as_array().unwrap_or(&vec![]) {
                    let artists = song["artists"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .map(|v| text(&v["name"]))
                        .collect::<Vec<_>>();
                    if !lyrics::matches(
                        track,
                        &text(&song["name"]),
                        &artists,
                        number(
                            song.get("duration")
                                .or_else(|| song.get("dt"))
                                .unwrap_or(&Value::Null),
                        ) / 1000.,
                    ) {
                        continue;
                    }
                    if let Ok(result) = query(
                        &format!(
                            "https://music.163.com/api/song/lyric?os=pc&lv=-1&kv=-1&tv=-1&id={}",
                            text(&song["id"])
                        ),
                        &[],
                    ) {
                        if let Some(raw) = result["lrc"]["lyric"].as_str() {
                            if !lyrics::parse(raw).is_empty() {
                                return Ok(raw.into());
                            }
                        }
                    }
                }
            }
        }
        Ok(String::new())
    }
    pub fn download(
        &self,
        stream: &Stream,
        target: &Path,
        overwrite: bool,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<u64> {
        if target.exists() && !overwrite {
            return Err(Error::new("文件已存在"));
        }
        fs::create_dir_all(target.parent().unwrap_or(Path::new(".")))?;
        let temporary = target.with_file_name(format!(
            ".{}-{}-{}.part",
            target.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let mut received = 0;
            let mut expected = stream.size;
            let mut digest = Md5::new();
            let mut server_md5 = None;
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(Error::coded("cancelled", "已取消"));
                }
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(|| Error::coded("network", "音频下载超时"))?;
                let header = format!("bytes={received}-{}", received + 1048575);
                let response = self.request(
                    &stream.url,
                    None,
                    &[("Range", &header)],
                    remaining.min(Duration::from_secs(12)),
                )?;
                let segment = if response.status() == 206 {
                    let (start, end, total) =
                        range(response.header("Content-Range").unwrap_or_default())
                            .ok_or_else(|| Error::new("服务器返回错误分块"))?;
                    if start != received || end < start || end >= total {
                        return Err(Error::new("服务器返回错误分块"));
                    }
                    expected = total;
                    Some(end - start + 1)
                } else if received == 0 {
                    expected = response
                        .header("Content-Length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(expected);
                    None
                } else {
                    return Err(Error::new("服务器不支持连续音频分块"));
                };
                if stream.size > 0 && expected > 0 && expected < stream.size * 8 / 10 {
                    return Err(Error::coded(
                        "incomplete_audio",
                        "服务器返回试听或不完整音频",
                    ));
                }
                if server_md5.is_none() {
                    server_md5 = response
                        .header("X-COS-META-MD5")
                        .or_else(|| response.header("Content-MD5"))
                        .map(str::to_string);
                }
                let mut reader = response.into_reader();
                let mut chunk_total = 0;
                let mut buffer = [0u8; 65536];
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return Err(Error::coded("cancelled", "已取消"));
                    }
                    let count = reader
                        .read(&mut buffer)
                        .map_err(|_| Error::coded("network", "音频传输中断"))?;
                    if count == 0 {
                        break;
                    }
                    if received + count as u64 > 1024 * 1024 * 1024
                        || segment.is_some_and(|n| chunk_total + count as u64 > n)
                    {
                        return Err(Error::new("音频分块长度异常"));
                    }
                    file.write_all(&buffer[..count])?;
                    digest.update(&buffer[..count]);
                    received += count as u64;
                    chunk_total += count as u64;
                    progress(received, expected);
                }
                if chunk_total == 0 || segment.is_some_and(|n| n != chunk_total) {
                    return Err(Error::coded("network", "音频分块传输未完成"));
                }
                if segment.is_none() || received >= expected {
                    break;
                }
            }
            if expected > 0 && received != expected {
                return Err(Error::coded("incomplete_audio", "音频传输长度不完整"));
            }
            if let Some(expected) = server_md5 {
                let value = digest.finalize();
                if expected.trim_matches('"').to_lowercase() != format!("{value:x}")
                    && expected.trim_matches('"') != STANDARD.encode(value)
                {
                    return Err(Error::new("音频 MD5 校验失败"));
                }
            }
            file.sync_all()?;
            if overwrite {
                fs::rename(&temporary, target)?;
            } else {
                fs::hard_link(&temporary, target)?;
            }
            Ok(received)
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn protocol_matches_existing_python_vector() {
        let raw = "<001><U_QRY>|<11,22>|<test>";
        assert_eq!(encode_query(raw), b"LkxQZR54BSdoPko3RgQvQjdtfz9UBT4lGkUh");
        assert_eq!(decode_reply(&encode_query(raw)).unwrap(), raw.as_bytes());
        assert!(decode_reply(b"").is_err());
    }
    #[test]
    fn legacy_dictionary_preserves_apostrophes_and_unicode() {
        let data =
            parse_json(br#"{'name': 'Don\'t', 'double': "John's", 'quote': 'a"b'}"#).unwrap();
        assert_eq!(data["name"], "Don't");
        assert_eq!(data["double"], "John's");
        assert_eq!(data["quote"], "a\"b");
    }
}

#[cfg(test)]
pub mod transfer_tests {
    use super::*;
    use std::{io::BufRead, net::TcpListener, thread};
    fn server(responses: Vec<(String, Vec<u8>)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/audio", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            for (headers, body) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                write!(socket, "HTTP/1.1 {headers}\r\nConnection: close\r\n\r\n").unwrap();
                let _ = socket.write_all(&body);
            }
        });
        (url, handle)
    }
    #[test]
    fn transfer_checks_ranges_checksum_and_overwrite() {
        let body = vec![42u8; 1048586];
        let md5 = format!("{:x}", Md5::digest(&body));
        let (url, handle) = server(vec![(format!("206 Partial Content\r\nContent-Range: bytes 0-1048575/1048586\r\nContent-Length: 1048576\r\nX-COS-META-MD5: {md5}"),body[..1048576].to_vec()),("206 Partial Content\r\nContent-Range: bytes 1048576-1048585/1048586\r\nContent-Length: 10".into(),body[1048576..].to_vec())]);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("song.mp3");
        let stream = Stream {
            url,
            quality: "320k".into(),
            size: body.len() as u64,
            extension: ".mp3".into(),
        };
        let provider = Provider::new(true);
        provider
            .download(&stream, &path, false, &AtomicBool::new(false), |_, _| {})
            .unwrap();
        handle.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), body);
        assert!(provider
            .download(&stream, &path, false, &AtomicBool::new(false), |_, _| {})
            .is_err());
    }
    #[test]
    fn bad_checksum_preserves_existing_file_and_cleans_parts() {
        let (url, handle) = server(vec![(
            "200 OK\r\nContent-Length: 3\r\nContent-MD5: wrong".into(),
            b"abc".to_vec(),
        )]);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("song.mp3");
        fs::write(&path, b"old").unwrap();
        assert!(Provider::new(true)
            .download(
                &Stream {
                    url,
                    quality: "128k".into(),
                    size: 3,
                    extension: ".mp3".into()
                },
                &path,
                true,
                &AtomicBool::new(false),
                |_, _| {}
            )
            .is_err());
        handle.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn wrong_range_and_short_preview_are_rejected() {
        for (headers, body) in [
            (
                "206 Partial Content\r\nContent-Range: bytes 1-3/100\r\nContent-Length: 3",
                b"abc".to_vec(),
            ),
            ("200 OK\r\nContent-Length: 3", b"ID3".to_vec()),
        ] {
            let (url, handle) = server(vec![(headers.into(), body)]);
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("song.mp3");
            assert!(Provider::new(true)
                .download(
                    &Stream {
                        url,
                        quality: "128k".into(),
                        size: 100,
                        extension: ".mp3".into()
                    },
                    &path,
                    false,
                    &AtomicBool::new(false),
                    |_, _| {}
                )
                .is_err());
            handle.join().unwrap();
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn cancellation_never_creates_final_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("song.mp3");
        assert!(Provider::new(true)
            .download(
                &Stream {
                    url: "http://127.0.0.1:1".into(),
                    quality: "128k".into(),
                    size: 3,
                    extension: ".mp3".into()
                },
                &path,
                false,
                &AtomicBool::new(true),
                |_, _| {}
            )
            .is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
