//! The only file that knows rmcp (pinned `~3.5.1`, which changes every week or two). It lists
//! the tools from `tools`, runs each call on a blocking thread through [`Server::call`], turns
//! that thread's progress lines into MCP progress notifications, and the reply into a result.
//!
//! Rules from `docs/spikes/mcp.md`: results are text (plus at most one PNG) with no
//! `structuredContent` and no `outputSchema`, because Claude Code would show the agent the JSON
//! instead of the text; errors are `isError` results with everything in the text; the tools set
//! `anthropic/alwaysLoad`; nothing happens in `initialize`, which Claude Code never calls.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, MetaObject, PaginatedRequestParams, ProgressNotificationParam,
    ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use super::{Reply, Server, Spec, tools};

const INSTRUCTIONS: &str = "Drive a GigaSculpt sculpting session. Find commands and their \
    arguments with gs_help; run one with gs_run, several as one undo step with gs_batch, and \
    read-only ones with gs_query; see the model with gs_look. A long command still running \
    after wait_ms answers with a job id: follow it with gs_job. Errors give a code, the bad \
    argument's path, a fix hint and a valid example.";

struct Handler(Arc<Server>);

/// Serves MCP on stdin and stdout until the client leaves.
pub async fn serve(server: Server) -> Result<(), String> {
    let running = Handler(Arc::new(server))
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("cannot serve MCP: {e}"))?;
    running.waiting().await.map_err(|e| e.to_string())?;
    Ok(())
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        let version = env!("CARGO_PKG_VERSION");
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("gigasculpt", version))
            .with_instructions(INSTRUCTIONS)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let listed = tools::all().iter().map(tool).collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(listed)))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let server = Arc::clone(&self.0);
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let (lines, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
        let work = tokio::task::spawn_blocking(move || {
            server.call(&name, args, &mut |line| {
                lines.send(line).ok();
            })
        });
        tokio::pin!(work);
        let token = context.meta.get_progress_token();
        let mut sent = 0_u32;
        let reply = loop {
            tokio::select! {
                Some(line) = heard.recv() => {
                    if let Some(token) = &token {
                        sent += 1;
                        let note = ProgressNotificationParam::new(token.clone(), f64::from(sent))
                            .with_message(line);
                        context.peer.notify_progress(note).await.ok();
                    }
                }
                done = &mut work => break done,
            }
        };
        let reply = reply.unwrap_or_else(|e| Reply {
            text: format!("error: the tool call stopped: {e}"),
            png: None,
            is_error: true,
        });
        Ok(result(reply).into())
    }
}

/// A tool as MCP lists it.
fn tool(spec: &Spec) -> Tool {
    let schema = match (spec.schema)() {
        Value::Object(schema) => schema,
        _ => JsonObject::new(),
    };
    let mut meta = JsonObject::new();
    meta.insert("anthropic/alwaysLoad".into(), Value::Bool(true));
    Tool::new(spec.name, spec.description, Arc::new(schema)).with_meta(MetaObject(meta))
}

/// A reply as an MCP result: the text, then the image if there is one.
fn result(reply: Reply) -> CallToolResult {
    let mut content = vec![ContentBlock::text(reply.text)];
    content.extend(reply.png.map(|png| ContentBlock::image(png, "image/png")));
    if reply.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}
