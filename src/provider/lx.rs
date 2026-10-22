//! LX musicUrl protocol hosted in a bounded, isolated QuickJS context.
use crate::model::{clean, Error, Result, Track};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use md5::{Digest, Md5};
use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

pub const MAX_SCRIPT: u64 = 2 * 1024 * 1024;
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;

type HttpReply = (u64, Result<Value>);
struct HttpHost {
    tx: Sender<HttpReply>,
    rx: Receiver<HttpReply>,
    closed: Arc<AtomicBool>,
    count: usize,
}
impl HttpHost {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            closed: Arc::new(AtomicBool::new(false)),
            count: 0,
        }
    }
}
impl Drop for HttpHost {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}
static HTTP_POOL: OnceLock<crate::pool::WorkerPool> = OnceLock::new();

pub fn metadata(script: &str) -> Value {
    let mut data = serde_json::Map::new();
    for key in ["name", "description", "version", "author", "homepage"] {
        if let Some(value) = script.lines().take(50).find_map(|line| {
            line.trim()
                .trim_start_matches('*')
                .trim()
                .strip_prefix(&format!("@{key} "))
        }) {
            data.insert(
                key.into(),
                json!(clean(value).chars().take(200).collect::<String>()),
            );
        }
    }
    Value::Object(data)
}

pub fn inspect(script: &str, agent: &ureq::Agent) -> Result<Value> {
    execute(
        script,
        None,
        agent,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(3),
    )
}

pub fn resolve(
    script: &str,
    track: &Track,
    quality: &str,
    agent: &ureq::Agent,
    cancelled: Arc<AtomicBool>,
) -> Result<String> {
    let id = track
        .id
        .strip_prefix("kuwo:")
        .ok_or_else(|| Error::new("LX 适配目前支持酷我歌曲 ID"))?;
    let input = json!({"source":"kw", "action":"musicUrl", "info":{
        "type":quality, "musicInfo":{"id":id,"songmid":id,"mid":id,"source":"kw",
        "name":track.title,"singer":track.artist,"albumName":track.album,
        "interval":crate::model::clock(track.duration),"duration":track.duration}
    }});
    let value = execute(
        script,
        Some(input),
        agent,
        cancelled,
        Duration::from_secs(10),
    )?;
    value
        .as_str()
        .filter(|s| s.len() < 16384 && (s.starts_with("https://") || s.starts_with("http://")))
        .map(str::to_owned)
        .ok_or_else(|| Error::coded("source_unavailable", "LX 音源未返回 HTTP 音频地址"))
}

fn js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> Error {
    let message = if error.is_exception() {
        let value = ctx.catch();
        value
            .as_exception()
            .map(|e| e.message().unwrap_or_default())
            .or_else(|| value.as_string().and_then(|s| s.to_string().ok()))
            .unwrap_or_else(|| "脚本异常或超出资源限制".into())
    } else {
        error.to_string()
    };
    Error::coded(
        "source_script",
        format!(
            "LX 脚本：{}",
            clean(&message).chars().take(300).collect::<String>()
        ),
    )
}

fn execute(
    script: &str,
    input: Option<Value>,
    agent: &ureq::Agent,
    cancelled: Arc<AtomicBool>,
    budget: Duration,
) -> Result<Value> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::coded("cancelled", "已取消音源请求"));
    }
    if script.len() as u64 > MAX_SCRIPT {
        return Err(Error::new("音源脚本超过 2 MiB"));
    }
    let runtime = Runtime::new().map_err(|e| Error::new(e.to_string()))?;
    runtime.set_memory_limit(32 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let deadline = Instant::now() + budget;
    let token = cancelled.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        token.load(Ordering::Relaxed) || Instant::now() >= deadline
    })));
    let context = Context::full(&runtime).map_err(|e| Error::new(e.to_string()))?;
    let mut host = HttpHost::new();
    context.with(|ctx| {
        let setup = (|| -> rquickjs::Result<()> {
            ctx.globals().set(
                "__lxUtil",
                Function::new(ctx.clone(), |ctx: Ctx<'_>, op: String, input: String| {
                    util(&op, &input).map_err(|e| Exception::throw_message(&ctx, &e.message))
                })?,
            )?;
            ctx.eval::<(), _>(include_str!("lx_bridge.js"))?;
            ctx.eval::<(), _>(format!("lx.currentScriptInfo = {}", json!({
                "name":metadata(script)["name"],"version":metadata(script)["version"],
                "author":metadata(script)["author"],"description":metadata(script)["description"],
                "homepage":metadata(script)["homepage"],"rawScript":script
            })))?;
            ctx.eval::<(), _>(script)?;
            Ok(())
        })();
        setup.map_err(|e| js_error(&ctx, e))?;
        pump(&ctx, agent, &cancelled, deadline, false, &mut host)?;
        let ready: String = ctx
            .eval("JSON.stringify(__lxState.ready)")
            .map_err(|e| js_error(&ctx, e))?;
        let ready: Value = serde_json::from_str(&ready)?;
        let kw = &ready["sources"]["kw"];
        if !kw["actions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "musicUrl"))
            || !kw["qualitys"].is_array()
        {
            return Err(Error::coded(
                "source_unsupported",
                "此 LX 音源未声明支持酷我 musicUrl，尚不支持其他平台歌曲 ID",
            ));
        }
        if let Some(input) = input {
            if !kw["qualitys"]
                .as_array()
                .unwrap()
                .contains(&input["info"]["type"])
            {
                return Err(Error::coded("source_unsupported", "此音源不支持所选音质"));
            }
            ctx.eval::<(), _>(format!("__lxStart({input})"))
                .map_err(|e| js_error(&ctx, e))?;
            pump(&ctx, agent, &cancelled, deadline, true, &mut host)?;
            let error: String = ctx.eval("__lxState.error").map_err(|e| js_error(&ctx, e))?;
            if !error.is_empty() {
                return Err(Error::coded("source_unavailable", clean(&error)));
            }
            let value: String = ctx
                .eval("JSON.stringify(__lxState.result ?? null)")
                .map_err(|e| js_error(&ctx, e))?;
            Ok(serde_json::from_str(&value)?)
        } else {
            Ok(ready)
        }
    })
}

fn pump(
    ctx: &Ctx<'_>,
    agent: &ureq::Agent,
    cancelled: &Arc<AtomicBool>,
    deadline: Instant,
    resolving: bool,
    host: &mut HttpHost,
) -> Result<()> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::coded("cancelled", "已取消音源请求"));
        }
        if Instant::now() >= deadline {
            return Err(Error::coded("source_timeout", "LX 音源执行超时"));
        }
        // Bound microtasks too: an infinite Promise chain must not starve the deadline.
        for _ in 0..100 {
            if !ctx.execute_pending_job() {
                break;
            }
        }
        let done: bool = ctx
            .eval(if resolving {
                "__lxState.done"
            } else {
                "__lxState.ready !== null"
            })
            .map_err(|e| js_error(ctx, e))?;
        if done {
            return Ok(());
        }
        ctx.eval::<(), _>("__lxTick()")
            .map_err(|e| js_error(ctx, e))?;
        let pending: String = ctx
            .eval("__lxTakeRequests()")
            .map_err(|e| js_error(ctx, e))?;
        let pending: Vec<Value> = serde_json::from_str(&pending)?;
        for request in pending {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::coded("cancelled", "已取消音源请求"));
            }
            host.count += 1;
            if host.count > 32 {
                return Err(Error::new("LX 单次操作请求超过 32 次"));
            }
            let agent = agent.clone();
            let cancelled = cancelled.clone();
            let closed = host.closed.clone();
            let tx = host.tx.clone();
            HTTP_POOL
                .get_or_init(|| crate::pool::WorkerPool::new(4))
                .execute(move || {
                    if closed.load(Ordering::Relaxed) || cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return;
                    }
                    let result = http(&agent, &request, remaining, &cancelled, &closed);
                    let _ = tx.send((request["id"].as_u64().unwrap_or_default(), result));
                });
        }
        for (id, result) in host.rx.try_iter() {
            let (error, response) = match result {
                Ok(value) => (Value::Null, value),
                Err(e) => (json!(e.message), Value::Null),
            };
            ctx.eval::<(), _>(format!("__lxReply({}, {}, {})", id, error, response))
                .map_err(|e| js_error(ctx, e))?;
            // Give the first fulfilled request a chance to settle Promise.any before another HTTP call.
            for _ in 0..100 {
                if !ctx.execute_pending_job() {
                    break;
                }
            }
            if ctx
                .eval::<bool, _>(if resolving {
                    "__lxState.done"
                } else {
                    "__lxState.ready !== null"
                })
                .map_err(|e| js_error(ctx, e))?
            {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn http(
    agent: &ureq::Agent,
    request: &Value,
    remaining: Duration,
    cancelled: &AtomicBool,
    closed: &AtomicBool,
) -> Result<Value> {
    let url = request["url"].as_str().unwrap_or_default();
    if url.len() > 16384 || !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(Error::new("音源仅允许 HTTP/HTTPS 请求"));
    }
    let options = &request["options"];
    let method = options["method"].as_str().unwrap_or("GET").to_uppercase();
    if !["GET", "POST", "PUT", "DELETE", "HEAD", "PATCH"].contains(&method.as_str()) {
        return Err(Error::new("不支持的 HTTP 方法"));
    }
    if !options["formData"].is_null() {
        return Err(Error::new("LX 兼容层暂不支持 formData"));
    }
    let timeout = Duration::from_millis(options["timeout"].as_u64().unwrap_or(8000).clamp(1, 8000))
        .min(remaining);
    let mut req = agent
        .request(&method, url)
        .timeout(timeout)
        .set("User-Agent", "wocli-music/LX");
    if let Some(headers) = options["headers"].as_object() {
        for (k, v) in headers {
            if let Some(v) = v.as_str() {
                req = req.set(k, v);
            }
        }
    }
    let response = if let Some(form) = options["form"].as_object() {
        let fields: Vec<(String, String)> = form
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect();
        req.send_form(
            &fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>(),
        )
    } else if !options["body"].is_null() {
        let body = options["body"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| options["body"].to_string());
        if body.len() as u64 > MAX_RESPONSE {
            return Err(Error::new("音源请求体过大"));
        }
        req.send_string(&body)
    } else {
        req.call()
    };
    let response = match response {
        Ok(r) | Err(ureq::Error::Status(_, r)) => r,
        Err(_) => return Err(Error::coded("network", "音源 HTTP 请求失败")),
    };
    let status = response.status();
    let headers: serde_json::Map<String, Value> = response
        .headers_names()
        .into_iter()
        .map(|k| {
            let v = json!(response.header(&k));
            (k, v)
        })
        .collect();
    let mut data = Vec::new();
    let mut reader = response.into_reader().take(MAX_RESPONSE + 1);
    let mut chunk = [0u8; 8192];
    loop {
        if cancelled.load(Ordering::Relaxed) || closed.load(Ordering::Relaxed) {
            return Err(Error::coded("cancelled", "已取消"));
        }
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    if data.len() as u64 > MAX_RESPONSE {
        return Err(Error::new("音源响应超过 2 MiB"));
    }
    let text = String::from_utf8_lossy(&data);
    let body = serde_json::from_slice::<Value>(&data).unwrap_or_else(|_| json!(text));
    Ok(json!({"statusCode":status,"status":status,"headers":headers,"body":body,"raw":text}))
}

fn util(op: &str, input: &str) -> Result<String> {
    if input.len() > 8 * 1024 * 1024 {
        return Err(Error::new("音源工具输入过大"));
    }
    match op {
        "md5" => Ok(format!("{:x}", Md5::digest(input.as_bytes()))),
        "random" => {
            let n: usize = input.parse().map_err(|_| Error::new("无效随机字节数"))?;
            if n > 65536 {
                return Err(Error::new("随机字节数过大"));
            }
            Ok(serde_json::to_string(
                &(0..n).map(|_| rand::random::<u8>()).collect::<Vec<_>>(),
            )?)
        }
        "decode" | "encode" => {
            let value: Value = serde_json::from_str(input)?;
            let encoding = value["encoding"].as_str().unwrap_or("utf8");
            if op == "decode" {
                let text = value["data"].as_str().unwrap_or_default();
                let bytes = match encoding {
                    "base64" => STANDARD
                        .decode(text)
                        .map_err(|_| Error::new("无效 Base64"))?,
                    "hex" => {
                        if !text.len().is_multiple_of(2) || !text.is_ascii() {
                            return Err(Error::new("无效十六进制"));
                        }
                        (0..text.len())
                            .step_by(2)
                            .map(|i| {
                                u8::from_str_radix(&text[i..i + 2], 16)
                                    .map_err(|_| Error::new("无效十六进制"))
                            })
                            .collect::<Result<Vec<_>>>()?
                    }
                    "utf8" | "utf-8" => text.as_bytes().to_vec(),
                    _ => return Err(Error::new("不支持的字符编码")),
                };
                Ok(serde_json::to_string(&bytes)?)
            } else {
                let bytes: Vec<u8> = serde_json::from_value(value["data"].clone())?;
                Ok(match encoding {
                    "base64" => STANDARD.encode(bytes),
                    "hex" => bytes.iter().map(|b| format!("{b:02x}")).collect(),
                    "utf8" | "utf-8" => String::from_utf8_lossy(&bytes).into_owned(),
                    _ => return Err(Error::new("不支持的字符编码")),
                })
            }
        }
        "inflate" | "deflate" => {
            let data: Vec<u8> = serde_json::from_str(input)?;
            let mut output = Vec::new();
            if op == "inflate" {
                ZlibDecoder::new(data.as_slice())
                    .take(MAX_RESPONSE + 1)
                    .read_to_end(&mut output)?;
            } else {
                let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
                e.write_all(&data)?;
                output = e.finish()?;
            }
            if output.len() as u64 > MAX_RESPONSE {
                return Err(Error::new("压缩输出过大"));
            }
            Ok(serde_json::to_string(&output)?)
        }
        _ => Err(Error::new("不支持的 LX 工具")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const INIT: &str =
        "lx.send(lx.EVENT_NAMES.inited,{sources:{kw:{actions:['musicUrl'],qualitys:['128k']}}});";
    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new().try_proxy_from_env(false).build()
    }
    #[test]
    fn lx_promises_timers_and_utils_work_without_host_access() {
        let script = format!(
            r#"/**
 * @name 测试音源
 */
{INIT}
lx.on('request', async ({{info}}) => {{
  if(typeof process !== 'undefined' || typeof require !== 'undefined') throw new Error('host access');
  if(lx.utils.crypto.md5('abc') !== '900150983cd24fb0d6963f7d28e17f72') throw new Error('md5');
  const b=lx.utils.buffer.from('你好');
  if(lx.utils.buffer.bufToString(b)!=='你好') throw new Error('buffer');
  const zipped=await lx.utils.zlib.deflate(b);
  if(lx.utils.buffer.bufToString(await lx.utils.zlib.inflate(zipped))!=='你好') throw new Error('zlib');
  await new Promise(r=>setTimeout(r,5));
  return 'https://example.test/'+info.musicInfo.songmid+'.mp3';
}});"#
        );
        assert_eq!(metadata(&script)["name"], "测试音源");
        assert!(inspect(&script, &agent()).is_ok());
        let track = Track {
            id: "kuwo:123".into(),
            ..Default::default()
        };
        assert_eq!(
            resolve(
                &script,
                &track,
                "128k",
                &agent(),
                Arc::new(AtomicBool::new(false))
            )
            .unwrap(),
            "https://example.test/123.mp3"
        );
    }
    #[test]
    fn cpu_and_microtask_loops_are_bounded_and_cancellation_is_reported() {
        for script in ["while(true){}".to_string(),format!("{INIT} lx.on('request',()=>new Promise(()=>{{const loop=()=>Promise.resolve().then(loop);loop();}}));")]{
            let input=script.contains("lx.on").then(||json!({"source":"kw","action":"musicUrl","info":{"type":"128k"}}));
            let start=Instant::now();
            assert!(execute(&script,input,&agent(),Arc::new(AtomicBool::new(false)),Duration::from_millis(60)).is_err());
            assert!(start.elapsed()<Duration::from_secs(2));
        }
        assert_eq!(
            execute(
                INIT,
                None,
                &agent(),
                Arc::new(AtomicBool::new(true)),
                Duration::from_secs(1)
            )
            .unwrap_err()
            .code,
            "cancelled"
        );
        assert_eq!(
            inspect(
                "lx.send('inited',{sources:{tx:{actions:['musicUrl'],qualitys:['128k']}}})",
                &agent()
            )
            .unwrap_err()
            .code,
            "source_unsupported"
        );
    }
    #[test]
    fn unknown_protocol_and_engine_errors_are_explicit() {
        assert!(inspect("throw new Error('syntax fixture');", &agent())
            .unwrap_err()
            .message
            .contains("syntax fixture"));
        let s = format!("{INIT} lx.on('request',()=>Promise.resolve('file:///etc/passwd'));");
        assert!(resolve(
            &s,
            &Track {
                id: "kuwo:1".into(),
                ..Default::default()
            },
            "128k",
            &agent(),
            Arc::new(AtomicBool::new(false))
        )
        .is_err());
    }
    #[test]
    fn slow_http_does_not_block_script_timers_or_promise_races() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if let Ok((mut socket, _)) = listener.accept() {
                    thread::sleep(Duration::from_millis(500));
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                    );
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let script=format!("{INIT} lx.on('request',()=>Promise.race([new Promise(r=>lx.request('{address}',{{}},()=>r('https://slow.test/a.mp3'))),new Promise(r=>setTimeout(()=>r('https://timer.test/a.mp3'),20))]));");
        let start = Instant::now();
        let result = resolve(
            &script,
            &Track {
                id: "kuwo:123".into(),
                ..Default::default()
            },
            "128k",
            &agent(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(result, "https://timer.test/a.mp3");
        assert!(start.elapsed() < Duration::from_millis(400));
        server.join().unwrap();
    }
}
