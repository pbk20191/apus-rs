//! An MCP server over HTTP where nothing owns a thread but the system:
//! - transport: Network.framework (C API, blocks delivered on the main queue)
//! - protocol: rmcp 3.10 (`local` feature), its tokio runtime a guest of the main run loop (no tokio thread)
//! - executor: async-swift-rs `run_main` on the main run loop; heavy work goes to dispatch queues
//!
//! POST /mcp with one JSON-RPC message per request; the answer comes back as application/json.
//! One session at a time (a new `initialize` replaces it). No GET stream, so server-initiated messages are dropped.
use block2::RcBlock;
use objc2_core_foundation::{kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFRetained, CFRunLoop, CFRunLoopTimer};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ClientJsonRpcMessage, JsonRpcMessage, ServerCapabilities, ServerConfig, ServerJsonRpcMessage};
use rmcp::{schemars, tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CString};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

// ============================================================================================================
// The guest runtime: three run loop timers give tokio its turns.
// ============================================================================================================
thread_local! {
    static RT: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) };
    static TIMERS: RefCell<Vec<CFRetained<CFRunLoopTimer>>> = const { RefCell::new(Vec::new()) };
}
static TURNS: AtomicUsize = AtomicUsize::new(0);
const FAR: f64 = 1e10;
const KICK: usize = 0; // one turn right after each poll of the LocalSet
const FAST: usize = 1; // every 50 ms while a request is in flight
const SLOW: usize = 2; // every second while a session is alive: a forgotten FAST can delay timers, never hang them

extern "C-unwind" fn turn(_timer: *mut CFRunLoopTimer, _info: *mut std::ffi::c_void) {
    TURNS.fetch_add(1, SeqCst);
    RT.with(|rt| rt.borrow().as_ref().unwrap().block_on(tokio::task::yield_now()));
}
fn arm(which: usize, on: bool) {
    TIMERS.with(|t| t.borrow()[which].set_next_fire_date(if on { CFAbsoluteTimeGetCurrent() } else { FAR }));
}
fn install_timers() {
    for interval in [FAR, 0.05, 1.0] {
        let timer = unsafe { CFRunLoopTimer::new(None, FAR, interval, 0, 0, Some(turn), std::ptr::null_mut()) }.unwrap();
        CFRunLoop::main().unwrap().add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes });
        TIMERS.with(|t| t.borrow_mut().push(timer));
    }
}
struct KickAfterPoll<F>(F);
impl<F: std::future::Future + Unpin> std::future::Future for KickAfterPoll<F> {
    type Output = F::Output;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        let result = std::pin::Pin::new(&mut self.0).poll(cx);
        arm(KICK, true);
        result
    }
}

// ============================================================================================================
// Network.framework: a loopback TCP listener; every callback is delivered on the main queue.
// ============================================================================================================
type Nw = *mut c_void;
#[link(name = "Network", kind = "framework")]
extern "C" {
    static _nw_parameters_configure_protocol_default_configuration: *mut c_void;
    static _nw_parameters_configure_protocol_disable: *mut c_void;
    static _nw_content_context_default_message: Nw;
    static _dispatch_main_q: c_void;
    fn nw_parameters_create_secure_tcp(configure_tls: *mut c_void, configure_tcp: *mut c_void) -> Nw;
    fn nw_parameters_set_local_endpoint(parameters: Nw, endpoint: Nw);
    fn nw_parameters_set_reuse_local_address(parameters: Nw, reuse: bool);
    fn nw_endpoint_create_host(hostname: *const c_char, port: *const c_char) -> Nw;
    fn nw_listener_create(parameters: Nw) -> Nw;
    fn nw_listener_set_queue(listener: Nw, queue: *const c_void);
    fn nw_listener_set_state_changed_handler(listener: Nw, handler: &block2::Block<dyn Fn(i32, Nw)>);
    fn nw_listener_set_new_connection_handler(listener: Nw, handler: &block2::Block<dyn Fn(Nw)>);
    fn nw_listener_start(listener: Nw);
    fn nw_listener_get_port(listener: Nw) -> u16;
    fn nw_connection_set_queue(connection: Nw, queue: *const c_void);
    fn nw_connection_start(connection: Nw);
    fn nw_connection_cancel(connection: Nw);
    fn nw_connection_receive(connection: Nw, min: u32, max: u32, completion: &block2::Block<dyn Fn(*mut c_void, Nw, u8, Nw)>);
    fn nw_connection_send(connection: Nw, content: *mut c_void, context: Nw, is_complete: bool, completion: &block2::Block<dyn Fn(Nw)>);
    fn nw_retain(object: Nw) -> Nw;
    fn nw_release(object: Nw);
    fn dispatch_data_create(buffer: *const u8, size: usize, queue: *const c_void, destructor: *const c_void) -> *mut c_void;
    fn dispatch_data_create_map(data: *mut c_void, buffer: *mut *const u8, size: *mut usize) -> *mut c_void;
    fn dispatch_release(object: *mut c_void);
}
fn main_queue() -> *const c_void { &raw const _dispatch_main_q }

/// One accepted TCP connection.
struct Connection(Nw);
impl Drop for Connection {
    fn drop(&mut self) { unsafe { nw_connection_cancel(self.0); nw_release(self.0) } }
}
impl Connection {
    /// The next chunk of bytes; `None` when the peer closed or the connection failed.
    async fn read(&self) -> Option<Vec<u8>> {
        let (tx, rx) = oneshot::channel();
        let tx = Cell::new(Some(tx));
        let done = RcBlock::new(move |content: *mut c_void, _context: Nw, _complete: u8, error: Nw| {
            let mut bytes = None;
            if !content.is_null() && error.is_null() {
                let (mut start, mut size) = (std::ptr::null(), 0usize);
                unsafe {
                    let mapped = dispatch_data_create_map(content, &mut start, &mut size);
                    bytes = Some(std::slice::from_raw_parts(start, size).to_vec());
                    dispatch_release(mapped);
                }
            }
            if let Some(tx) = tx.take() { let _ = tx.send(bytes); }
        });
        unsafe { nw_connection_receive(self.0, 1, 64 * 1024, &done) };
        rx.await.ok().flatten()
    }
    async fn write(&self, bytes: &[u8]) -> bool {
        let (tx, rx) = oneshot::channel();
        let tx = Cell::new(Some(tx));
        let done = RcBlock::new(move |error: Nw| { if let Some(tx) = tx.take() { let _ = tx.send(error.is_null()); } });
        unsafe {
            let data = dispatch_data_create(bytes.as_ptr(), bytes.len(), std::ptr::null(), std::ptr::null()); // copies
            nw_connection_send(self.0, data, _nw_content_context_default_message, false, &done);
            dispatch_release(data);
        }
        rx.await.unwrap_or(false)
    }
}

/// Starts listening on 127.0.0.1 (system-assigned port). Connections arrive on the returned channel.
fn listen() -> (oneshot::Receiver<u16>, mpsc::UnboundedReceiver<Connection>) {
    let (port_tx, port_rx) = oneshot::channel();
    let (conn_tx, conn_rx) = mpsc::unbounded_channel();
    unsafe {
        let parameters = nw_parameters_create_secure_tcp(_nw_parameters_configure_protocol_disable, _nw_parameters_configure_protocol_default_configuration);
        let (host, port) = (CString::new("127.0.0.1").unwrap(), CString::new("0").unwrap());
        let endpoint = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
        nw_parameters_set_local_endpoint(parameters, endpoint);
        nw_parameters_set_reuse_local_address(parameters, true);
        let listener = nw_listener_create(parameters); // kept for the life of the process
        nw_release(endpoint);
        nw_release(parameters);
        nw_listener_set_queue(listener, main_queue());
        let port_tx = Cell::new(Some(port_tx));
        let listener_address = listener as usize;
        nw_listener_set_state_changed_handler(listener, &RcBlock::new(move |state: i32, _error: Nw| {
            if state == 2 { if let Some(tx) = port_tx.take() { let _ = tx.send(nw_listener_get_port(listener_address as Nw)); } }
            if state == 3 { eprintln!("listener failed"); std::process::exit(1) }
        }));
        nw_listener_set_new_connection_handler(listener, &RcBlock::new(move |connection: Nw| {
            nw_retain(connection);
            nw_connection_set_queue(connection, main_queue());
            nw_connection_start(connection);
            let _ = conn_tx.send(Connection(connection));
        }));
        nw_listener_start(listener);
    }
    (port_rx, conn_rx)
}

// ============================================================================================================
// HTTP/1.1: just enough for POST /mcp with Content-Length, keep-alive included.
// ============================================================================================================
struct Request { method: String, path: String, body: Vec<u8> }
async fn read_request(connection: &Connection, buffer: &mut Vec<u8>) -> Option<Request> {
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut parsed = httparse::Request::new(&mut headers);
        if let Ok(httparse::Status::Complete(head)) = parsed.parse(buffer) {
            let length = parsed.headers.iter().find(|h| h.name.eq_ignore_ascii_case("content-length"))
                .and_then(|h| std::str::from_utf8(h.value).ok()?.trim().parse::<usize>().ok()).unwrap_or(0);
            if buffer.len() >= head + length {
                let request = Request { method: parsed.method?.to_string(), path: parsed.path?.to_string(), body: buffer[head..head + length].to_vec() };
                buffer.drain(..head + length);
                return Some(request);
            }
        }
        buffer.extend(connection.read().await?);
    }
}
async fn respond(connection: &Connection, status: &str, session: Option<&str>, body: &[u8]) -> bool {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    if !body.is_empty() { head += "Content-Type: application/json\r\n" }
    if let Some(id) = session { head += &format!("Mcp-Session-Id: {id}\r\n") }
    head += "\r\n";
    connection.write(&[head.as_bytes(), body].concat()).await
}

// ============================================================================================================
// The MCP session: rmcp's service loop behind a channel transport; answers are routed back by request id.
// ============================================================================================================
type Pending = Rc<RefCell<HashMap<String, oneshot::Sender<serde_json::Value>>>>;
#[derive(Clone)]
struct Session { id: String, to_server: mpsc::UnboundedSender<ClientJsonRpcMessage>, pending: Pending }
#[derive(Default)]
struct App { session: RefCell<Option<Session>>, sessions_started: Cell<u32> }

struct SessionTransport { incoming: mpsc::UnboundedReceiver<ClientJsonRpcMessage>, pending: Pending }
// `Transport: Send` is required even with rmcp's `local` feature; this value never leaves the main thread.
unsafe impl Send for SessionTransport {}
impl rmcp::transport::Transport<RoleServer> for SessionTransport {
    type Error = std::io::Error;
    fn send(&mut self, item: ServerJsonRpcMessage) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let is_answer = matches!(item, JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_));
        let value = serde_json::to_value(&item).unwrap();
        match self.pending.borrow_mut().remove(&value["id"].to_string()) {
            Some(waiting) if is_answer => { let _ = waiting.send(value); }
            _ => eprintln!("  (dropped a server-initiated message: {})", value["method"]),
        }
        if self.pending.borrow().is_empty() { arm(FAST, false) }
        std::future::ready(Ok(()))
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> { self.incoming.recv().await }
    async fn close(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

fn start_session(app: &Rc<App>) -> Session {
    app.sessions_started.set(app.sessions_started.get() + 1);
    let (to_server, incoming) = mpsc::unbounded_channel();
    let session = Session { id: format!("s{}", app.sessions_started.get()), to_server, pending: Pending::default() };
    let transport = SessionTransport { incoming, pending: session.pending.clone() };
    let id = session.id.clone();
    arm(SLOW, true);
    tokio::task::spawn_local(async move {
        let server = Desktop { tool_router: Desktop::tool_router(), calls: Rc::new(Cell::new(0)) };
        match server.serve(transport).await {
            Ok(running) => eprintln!("  session {id} ended: {:?}", running.waiting().await),
            Err(error) => eprintln!("  session {id} failed to initialize: {error}"),
        }
    });
    *app.session.borrow_mut() = Some(session.clone()); // replacing drops the old sender, which closes the old session
    session
}

async fn handle_connection(connection: Connection, app: Rc<App>) {
    let mut buffer = Vec::new();
    while let Some(request) = read_request(&connection, &mut buffer).await {
        let ok = match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/mcp") => handle_post(&connection, &app, &request.body).await,
            ("GET", "/stats") => {
                let body = serde_json::json!({ "threads": thread_count(), "turns": TURNS.load(SeqCst), "on_main": async_swift_rs::is_main_thread() });
                respond(&connection, "200 OK", None, body.to_string().as_bytes()).await
            }
            ("DELETE", "/mcp") => { *app.session.borrow_mut() = None; arm(SLOW, false); respond(&connection, "204 No Content", None, b"").await }
            _ => respond(&connection, "404 Not Found", None, b"").await,
        };
        if !ok { break }
    }
}

async fn handle_post(connection: &Connection, app: &Rc<App>, body: &[u8]) -> bool {
    let error = |code: i64, message: &str| serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":code,"message":message}}).to_string();
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return respond(connection, "400 Bad Request", None, error(-32700, "parse error").as_bytes()).await;
    };
    let Ok(message) = serde_json::from_value::<ClientJsonRpcMessage>(value.clone()) else {
        return respond(connection, "400 Bad Request", None, error(-32600, "not a client message").as_bytes()).await;
    };
    let existing = app.session.borrow().clone();
    let session = match (value["method"].as_str(), existing) {
        (Some("initialize"), _) => start_session(app),
        (_, Some(session)) => session,
        (_, None) => return respond(connection, "400 Bad Request", None, error(-32000, "no session: send initialize first").as_bytes()).await,
    };
    if !matches!(message, JsonRpcMessage::Request(_)) {
        let _ = session.to_server.send(message);
        return respond(connection, "202 Accepted", Some(&session.id), b"").await;
    }
    let (tx, rx) = oneshot::channel();
    session.pending.borrow_mut().insert(value["id"].to_string(), tx);
    arm(FAST, true);
    let _ = session.to_server.send(message);
    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(answer)) => respond(connection, "200 OK", Some(&session.id), answer.to_string().as_bytes()).await,
        _ => respond(connection, "504 Gateway Timeout", Some(&session.id), error(-32001, "no answer").as_bytes()).await,
    }
}

// ============================================================================================================
// The tools.
// ============================================================================================================
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SumRequest { a: i64, b: i64 }
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct WaitRequest { millis: u64 }
/// Not Send: main-thread state, the way a handler holding AppKit or accessibility objects would be.
struct Desktop { tool_router: ToolRouter<Self>, calls: Rc<Cell<u32>> }
#[tool_router]
impl Desktop {
    #[tool(description = "Adds two numbers")]
    fn sum(&self, Parameters(SumRequest { a, b }): Parameters<SumRequest>) -> String {
        self.calls.set(self.calls.get() + 1);
        format!("{} (handler on main: {})", a + b, async_swift_rs::is_main_thread())
    }
    #[tool(description = "Waits on a tokio timer, then reports")]
    async fn wait(&self, Parameters(WaitRequest { millis }): Parameters<WaitRequest>) -> String {
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(millis)).await;
        format!("asked {millis} ms, took {} ms", start.elapsed().as_millis())
    }
    #[tool(description = "Does heavy work on a dispatch queue, off the main thread")]
    async fn heavy(&self) -> String {
        self.calls.set(self.calls.get() + 1);
        let worker_on_main = async_swift_rs::dispatch::spawn_detached(async { std::thread::sleep(Duration::from_millis(40)); async_swift_rs::is_main_thread() }).await.unwrap();
        format!("worker on main: {worker_on_main}; back on main: {}; call #{}", async_swift_rs::is_main_thread(), self.calls.get())
    }
}
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Desktop {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions("nw + rmcp guest spike")
    }
}

fn thread_count() -> u32 {
    extern "C" { fn task_threads(task: u32, list: *mut *mut u32, count: *mut u32) -> i32; static mach_task_self_: u32; }
    let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
    unsafe { task_threads(mach_task_self_, &mut list, &mut count) };
    count
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let handle = rt.handle().clone();
    RT.with(|slot| *slot.borrow_mut() = Some(rt));
    install_timers();
    let _guard = handle.enter();
    let local = tokio::task::LocalSet::new();
    async_swift_rs::run_main(KickAfterPoll(Box::pin(async move {
        local.run_until(async move {
            let app = Rc::new(App::default());
            let (port, mut connections) = listen();
            println!("listening on 127.0.0.1:{}", port.await.unwrap());
            while let Some(connection) = connections.recv().await {
                tokio::task::spawn_local(handle_connection(connection, app.clone()));
            }
        }).await
    })));
}
