//! Local MCP adapter. Audio remains owned by the existing, detached player daemon.
mod backend;
mod catalog;
mod params;

use crate::{
    model::{Error, Result},
    store::Paths,
};
use backend::Backend;
use rmcp::{
    model::*,
    service::RequestContext,
    transport::{async_rw::AsyncRwTransport, stdio_pipes},
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const MAX_OUTPUT: usize = 512 * 1024;
#[derive(Clone)]
struct MusicServer {
    backend: Backend,
    slots: Arc<tokio::sync::Semaphore>,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
impl MusicServer {
    async fn execute(
        &self,
        name: String,
        args: Value,
        context: RequestContext<RoleServer>,
    ) -> Result<Value> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::coded("busy", "MCP 同时处理的请求已达 8 个，请稍后重试"))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(cancelled.clone());
        let backend = self.backend.clone();
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.execute(&name, args, &cancelled)
        });
        let value = tokio::select! {
            _=context.ct.cancelled()=>Err(Error::coded("cancelled","MCP 请求已取消；已提交的后台操作仍可能继续")),
            result=tokio::time::timeout(Duration::from_secs(45),work)=>match result{
                Ok(Ok(value))=>value,
                Ok(Err(_))=>Err(Error::coded("internal","MCP 工作线程异常")),
                Err(_)=>Err(Error::coded("timeout","MCP 请求等待超过 45 秒；已提交的后台操作仍可能继续")),
            }
        }?;
        if serde_json::to_vec(&value)?.len() > MAX_OUTPUT {
            return Err(Error::coded(
                "result_too_large",
                "结果超过 512 KiB，请缩小分页或查询范围",
            ));
        }
        Ok(value)
    }
}
const RESOURCES: &[(&str, &str, &str)] = &[
    ("wocli://status", "Playback status", "get_playback_status"),
    ("wocli://queue", "Queue (first 20 tracks)", "get_queue"),
    ("wocli://sources", "Music sources", "list_music_sources"),
    (
        "wocli://settings",
        "Player preferences",
        "get_player_config",
    ),
    (
        "wocli://downloads",
        "Downloads (newest 20)",
        "get_downloads",
    ),
];
impl ServerHandler for MusicServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("wocli-music",env!("CARGO_PKG_VERSION")))
            .with_instructions("Control the local wocli music player. Search then use exact track IDs; do not silently select ambiguous songs. Playback and download calls enqueue work: poll get_playback_status/get_downloads for the outcome. Queue positions are 1-based. Closing MCP leaves the audio daemon running. Source import and program updates are available only through the CLI.")
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        catalog::tools(self.backend.read_only)
            .into_iter()
            .find(|t| t.name == name)
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        if request.and_then(|p| p.cursor).is_some() {
            return Err(McpError::invalid_params("此工具列表没有分页游标", None));
        }
        Ok(ListToolsResult {
            tools: catalog::tools(self.backend.read_only),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        if self.get_tool(&request.name).is_none() {
            return Err(McpError::invalid_params(
                "未知工具或此工具在只读模式下未开放",
                None,
            ));
        }
        let args = Value::Object(request.arguments.unwrap_or_default());
        let result = match self.execute(request.name.to_string(), args, context).await {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(
                json!({"code":error.code,"error":error.message,"details":error.details}),
            ),
        };
        Ok(result.into())
    }
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, McpError> {
        if request.and_then(|p| p.cursor).is_some() {
            return Err(McpError::invalid_params("此资源列表没有分页游标", None));
        }
        Ok(ListResourcesResult {
            resources: RESOURCES
                .iter()
                .map(|(uri, name, _)| Resource::new(*uri, *name).with_mime_type("application/json"))
                .collect(),
            ..Default::default()
        })
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResponse, McpError> {
        let (_, _, tool) = RESOURCES
            .iter()
            .find(|(uri, _, _)| *uri == request.uri)
            .ok_or_else(|| McpError::resource_not_found("未知 wocli 资源", None))?;
        let value = self
            .execute((*tool).into(), json!({}), context)
            .await
            .map_err(|e| McpError::internal_error(e.message, Some(json!({"code":e.code}))))?;
        let mut content = ResourceContents::text(
            serde_json::to_string(&value)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            request.uri,
        );
        if let ResourceContents::TextResourceContents { mime_type, .. } = &mut content {
            *mime_type = Some("application/json".into());
        }
        Ok(ReadResourceResult::new(vec![content])
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private)
            .into())
    }
}

pub fn config(paths: &Paths, read_only: bool, direct: bool) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let mut args = vec![
        "mcp".to_string(),
        "serve".into(),
        "--home".into(),
        paths.home.to_string_lossy().into_owned(),
    ];
    if read_only {
        args.push("--read-only".into());
    }
    if direct {
        args.push("--direct".into());
    }
    Ok(json!({"mcpServers":{"wocli-music":{"command":exe,"args":args}}}))
}
pub fn tools(read_only: bool) -> Result<Value> {
    Ok(json!({"tools":catalog::tools(read_only)}))
}
pub fn run(paths: Paths, read_only: bool, direct: bool) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        // Eight tool workers plus blocking stdin/stdout on macOS and Windows.
        .max_blocking_threads(10)
        .enable_all()
        .build()?;
    let result=runtime.block_on(async move {
        let server=MusicServer{backend:Backend{paths,direct,read_only},slots:Arc::new(tokio::sync::Semaphore::new(8))};
        let (reader,writer)=stdio_pipes();
        let transport=AsyncRwTransport::<RoleServer,_,_>::new(reader,writer).with_max_line_length(64*1024);
        let running=match server.serve(transport).await {
            Ok(running)=>running,
            Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)
                | rmcp::service::ServerInitializeError::ExpectedInitializeRequest(None))=>return Ok(()),
            Err(error)=>return Err(Error::new(format!("MCP 连接初始化失败：{error}"))),
        };
        let token=running.cancellation_token();
        tokio::select!{
            result=running.waiting()=>{result.map_err(|e|Error::new(format!("MCP 服务退出异常：{e}")))?;},
            _=tokio::signal::ctrl_c()=>{token.cancel();},
        }
        Ok(())
    });
    // On macOS/Windows stdio may be backed by a blocking reader. Never wait forever on shutdown.
    runtime.shutdown_timeout(Duration::from_millis(500));
    result
}
