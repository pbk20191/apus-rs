//! Stateless mode: one rmcp service per request via `serve_directly` (no initialize handshake).
//! Plain tokio current_thread here: this checks rmcp's behaviour, not the guest runtime.
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ClientJsonRpcMessage, ServerCapabilities, ServerConfig, ServerJsonRpcMessage};
use rmcp::{schemars, tool, tool_handler, tool_router, RoleServer, ServerHandler};
use tokio::sync::{mpsc, oneshot};

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SumRequest { a: i64, b: i64 }
struct Desktop { tool_router: ToolRouter<Self> }
#[tool_router]
impl Desktop {
    #[tool(description = "Adds two numbers")]
    fn sum(&self, Parameters(SumRequest { a, b }): Parameters<SumRequest>) -> String { (a + b).to_string() }
}
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Desktop {
    fn get_info(&self) -> ServerConfig { ServerConfig::new(ServerCapabilities::builder().enable_tools().build()) }
}

/// Delivers one message, then stays open until the answer has been sent.
struct OneShot { message: Option<ClientJsonRpcMessage>, answer: Option<oneshot::Sender<ServerJsonRpcMessage>>, done: mpsc::UnboundedReceiver<()>, finished: mpsc::UnboundedSender<()> }
impl rmcp::transport::Transport<RoleServer> for OneShot {
    type Error = std::io::Error;
    fn send(&mut self, item: ServerJsonRpcMessage) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        if let Some(answer) = self.answer.take() { let _ = answer.send(item); }
        let _ = self.finished.send(());
        std::future::ready(Ok(()))
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        if let Some(message) = self.message.take() { return Some(message) }
        self.done.recv().await; // the answer is out: close
        None
    }
    async fn close(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

async fn stateless(json: serde_json::Value) -> String {
    let (answer, rx) = oneshot::channel();
    let (finished, done) = mpsc::unbounded_channel();
    let transport = OneShot { message: Some(serde_json::from_value(json).unwrap()), answer: Some(answer), done, finished };
    let running = rmcp::service::serve_directly(Desktop { tool_router: Desktop::tool_router() }, transport, None);
    let reply = match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
        Ok(Ok(message)) => serde_json::to_string(&message).unwrap(),
        _ => "(no answer within 2 s)".to_string(),
    };
    let end = tokio::time::timeout(std::time::Duration::from_secs(8), running.waiting()).await;
    format!("{}   [service end: {}]", &reply[..reply.len().min(230)], match end { Ok(r) => format!("{r:?}"), Err(_) => "still running after 8 s".into() })
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        let start = std::time::Instant::now();
        println!("initialize → {}", stateless(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"0"}}})).await);
        println!("tools/list → {}", stateless(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await);
        println!("tools/call → {}", stateless(serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sum","arguments":{"a":40,"b":2}}})).await);
        println!("ping       → {}", stateless(serde_json::json!({"jsonrpc":"2.0","id":4,"method":"ping"})).await);
        println!("four requests took {:?}", start.elapsed());
    });
}
