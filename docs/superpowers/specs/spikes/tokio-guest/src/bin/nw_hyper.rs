//! The same server as nw_mcp, with hyper doing HTTP:
//! - `NwIo` adapts a Network.framework connection to hyper's poll-based I/O. Received `dispatch_data` is queued
//!   without copying (dispatch_data_create_concat / dispatch_data_apply / dispatch_data_create_subrange); the one copy is into hyper's own buffer.
//! - GET /mcp opens an SSE stream that carries server-initiated messages.
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
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, Response, StatusCode};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
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
    fn dispatch_data_create_concat(first: *mut c_void, second: *mut c_void) -> *mut c_void;
    fn dispatch_data_create_subrange(data: *mut c_void, offset: usize, length: usize) -> *mut c_void;
    /// Calls `applier(region, offset, address, size)` for each contiguous region, in order, until it returns false.
    fn dispatch_data_apply(data: *mut c_void, applier: &block2::Block<dyn Fn(*mut c_void, usize, *const u8, usize) -> u8>) -> bool;
    fn dispatch_data_get_size(data: *mut c_void) -> usize;
    fn dispatch_retain(object: *mut c_void);
    fn dispatch_release(object: *mut c_void);
}
fn main_queue() -> *const c_void { &raw const _dispatch_main_q }

/// One accepted TCP connection.
struct Connection(Nw);
impl Drop for Connection {
    fn drop(&mut self) { unsafe { nw_connection_cancel(self.0); nw_release(self.0) } }
}

/// What the connection's callbacks and hyper's polls share. Main thread only.
struct IoState {
    /// Received and not yet consumed: one dispatch_data made of the chunks Network.framework handed over.
    queue: *mut c_void,
    receiving: bool,
    eof: bool,
    read_waker: Option<Waker>,
    /// Bytes handed to nw_connection_send whose completion has not arrived.
    unsent: usize,
    write_failed: bool,
    write_waker: Option<Waker>,
}
impl Drop for IoState {
    fn drop(&mut self) { if !self.queue.is_null() { unsafe { dispatch_release(self.queue) } } }
}
static BYTES_COPIED_IN: AtomicUsize = AtomicUsize::new(0);
const HIGH_WATER: usize = 256 * 1024;

/// hyper's view of a Network.framework connection.
struct NwIo { connection: Connection, state: Rc<RefCell<IoState>> }
impl NwIo {
    fn new(connection: Connection) -> Self {
        let state = IoState { queue: std::ptr::null_mut(), receiving: false, eof: false, read_waker: None, unsent: 0, write_failed: false, write_waker: None };
        NwIo { connection, state: Rc::new(RefCell::new(state)) }
    }
}
impl hyper::rt::Read for NwIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, mut buf: hyper::rt::ReadBufCursor<'_>) -> Poll<std::io::Result<()>> {
        let mut state = self.state.borrow_mut();
        if !state.queue.is_null() {
            unsafe {
                // Walk the queue's regions in place (dispatch_data_apply hands out each region's address) and fill
                // hyper's buffer: the one copy. Nothing is mapped or flattened.
                let total = dispatch_data_get_size(state.queue);
                let cursor = &mut buf as *mut hyper::rt::ReadBufCursor<'_> as usize; // used only during the synchronous apply
                let copied = Rc::new(Cell::new(0usize));
                let counter = copied.clone();
                let fill = RcBlock::new(move |_region: *mut c_void, _offset: usize, address: *const u8, size: usize| -> u8 {
                    let buf = &mut *(cursor as *mut hyper::rt::ReadBufCursor<'_>);
                    let count = size.min(buf.remaining());
                    buf.put_slice(std::slice::from_raw_parts(address, count));
                    counter.set(counter.get() + count);
                    (buf.remaining() > 0) as u8 // keep going while hyper has room
                });
                dispatch_data_apply(state.queue, &fill);
                let count = copied.get();
                BYTES_COPIED_IN.fetch_add(count, SeqCst);
                let rest = if count == total { std::ptr::null_mut() } else { dispatch_data_create_subrange(state.queue, count, total - count) };
                dispatch_release(state.queue);
                state.queue = rest;
            }
            return Poll::Ready(Ok(()));
        }
        if state.eof { return Poll::Ready(Ok(())) }
        state.read_waker = Some(cx.waker().clone());
        if !state.receiving {
            state.receiving = true;
            let shared = self.state.clone();
            let done = RcBlock::new(move |content: *mut c_void, _context: Nw, _complete: u8, error: Nw| {
                let mut state = shared.borrow_mut();
                state.receiving = false;
                if !content.is_null() && error.is_null() {
                    unsafe {
                        if state.queue.is_null() {
                            dispatch_retain(content);
                            state.queue = content;
                        } else {
                            let joined = dispatch_data_create_concat(state.queue, content); // no copy
                            dispatch_release(state.queue);
                            state.queue = joined;
                        }
                    }
                } else {
                    state.eof = true;
                }
                if let Some(waker) = state.read_waker.take() { waker.wake() }
            });
            unsafe { nw_connection_receive(self.connection.0, 1, 64 * 1024, &done) };
        }
        Poll::Pending
    }
}
impl hyper::rt::Write for NwIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<std::io::Result<usize>> {
        let mut state = self.state.borrow_mut();
        if state.write_failed { return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into())) }
        if state.unsent >= HIGH_WATER {
            state.write_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        state.unsent += bytes.len();
        let (shared, length) = (self.state.clone(), bytes.len());
        let done = RcBlock::new(move |error: Nw| {
            let mut state = shared.borrow_mut();
            state.unsent -= length;
            if !error.is_null() { state.write_failed = true }
            if let Some(waker) = state.write_waker.take() { waker.wake() }
        });
        unsafe {
            let data = dispatch_data_create(bytes.as_ptr(), bytes.len(), std::ptr::null(), std::ptr::null()); // copies: hyper only lends the bytes
            nw_connection_send(self.connection.0, data, _nw_content_context_default_message, false, &done);
            dispatch_release(data);
        }
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Poll::Ready(Ok(())) }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Poll::Ready(Ok(())) }
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
// The MCP session: rmcp's service loop behind a channel transport; answers are routed back by request id.
// ============================================================================================================
type Pending = Rc<RefCell<HashMap<String, oneshot::Sender<serde_json::Value>>>>;
type Stream = Rc<RefCell<Option<mpsc::UnboundedSender<Bytes>>>>;
#[derive(Clone)]
struct Session { id: String, to_server: mpsc::UnboundedSender<ClientJsonRpcMessage>, pending: Pending, stream: Stream }
#[derive(Default)]
struct App { session: RefCell<Option<Session>>, sessions_started: Cell<u32> }

struct SessionTransport { incoming: mpsc::UnboundedReceiver<ClientJsonRpcMessage>, pending: Pending, stream: Stream }
// `Transport: Send` is required even with rmcp's `local` feature; this value never leaves the main thread.
unsafe impl Send for SessionTransport {}
impl rmcp::transport::Transport<RoleServer> for SessionTransport {
    type Error = std::io::Error;
    fn send(&mut self, item: ServerJsonRpcMessage) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let is_answer = matches!(item, JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_));
        let value = serde_json::to_value(&item).unwrap();
        match self.pending.borrow_mut().remove(&value["id"].to_string()) {
            Some(waiting) if is_answer => { let _ = waiting.send(value); }
            _ => match self.stream.borrow().as_ref() {
                // Server-initiated: goes out on the GET stream as a server-sent event.
                Some(stream) => { let _ = stream.send(Bytes::from(format!("event: message\ndata: {value}\n\n"))); }
                None => eprintln!("  (no GET stream open: dropped {})", value["method"]),
            },
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
    let session = Session { id: format!("s{}", app.sessions_started.get()), to_server, pending: Pending::default(), stream: Stream::default() };
    let transport = SessionTransport { incoming, pending: session.pending.clone(), stream: session.stream.clone() };
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

type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, std::convert::Infallible>;
fn reply(status: StatusCode, session: Option<&str>, body: String) -> Response<Body> {
    let mut response = Response::builder().status(status);
    if !body.is_empty() { response = response.header("Content-Type", "application/json") }
    if let Some(id) = session { response = response.header("Mcp-Session-Id", id) }
    response.body(Full::new(Bytes::from(body)).boxed_unsync()).unwrap()
}
/// A response body fed from a channel: the SSE stream.
struct ChannelBody(mpsc::UnboundedReceiver<Bytes>);
impl http_body::Body for ChannelBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        self.0.poll_recv(cx).map(|chunk| chunk.map(|bytes| Ok(http_body::Frame::data(bytes))))
    }
}

async fn handle_request(request: Request<hyper::body::Incoming>, app: Rc<App>) -> Result<Response<Body>, std::convert::Infallible> {
    let path = request.uri().path().to_string();
    Ok(match (request.method().clone(), path.as_str()) {
        (Method::POST, "/mcp") => {
            let body = request.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            handle_post(&app, &body).await
        }
        (Method::GET, "/mcp") => {
            let Some(session) = app.session.borrow().clone() else { return Ok(reply(StatusCode::BAD_REQUEST, None, String::new())) };
            let (tx, rx) = mpsc::unbounded_channel();
            let _ = tx.send(Bytes::from_static(b": stream open\n\n"));
            *session.stream.borrow_mut() = Some(tx);
            Response::builder().header("Content-Type", "text/event-stream").header("Cache-Control", "no-cache")
                .header("Mcp-Session-Id", &session.id).body(ChannelBody(rx).boxed_unsync()).unwrap()
        }
        (Method::GET, "/stats") => reply(StatusCode::OK, None, serde_json::json!({
            "threads": thread_count(), "turns": TURNS.load(SeqCst), "on_main": async_swift_rs::is_main_thread(), "bytes_copied_in": BYTES_COPIED_IN.load(SeqCst) }).to_string()),
        (Method::DELETE, "/mcp") => { *app.session.borrow_mut() = None; arm(SLOW, false); reply(StatusCode::NO_CONTENT, None, String::new()) }
        _ => reply(StatusCode::NOT_FOUND, None, String::new()),
    })
}

async fn handle_post(app: &Rc<App>, body: &[u8]) -> Response<Body> {
    let error = |code: i64, message: &str| serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":code,"message":message}}).to_string();
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return reply(StatusCode::BAD_REQUEST, None, error(-32700, "parse error")) };
    let Ok(message) = serde_json::from_value::<ClientJsonRpcMessage>(value.clone()) else { return reply(StatusCode::BAD_REQUEST, None, error(-32600, "not a client message")) };
    let existing = app.session.borrow().clone();
    let session = match (value["method"].as_str(), existing) {
        (Some("initialize"), _) => start_session(app),
        (_, Some(session)) => session,
        (_, None) => return reply(StatusCode::BAD_REQUEST, None, error(-32000, "no session: send initialize first")),
    };
    if !matches!(message, JsonRpcMessage::Request(_)) {
        let _ = session.to_server.send(message);
        return reply(StatusCode::ACCEPTED, Some(&session.id), String::new());
    }
    let (tx, rx) = oneshot::channel();
    session.pending.borrow_mut().insert(value["id"].to_string(), tx);
    arm(FAST, true);
    let _ = session.to_server.send(message);
    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(answer)) => reply(StatusCode::OK, Some(&session.id), answer.to_string()),
        _ => reply(StatusCode::GATEWAY_TIMEOUT, Some(&session.id), error(-32001, "no answer")),
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
    #[tool(description = "Sends a server-initiated notification, then answers")]
    async fn announce(&self, peer: rmcp::Peer<RoleServer>) -> String {
        match peer.notify_resource_list_changed().await { Ok(()) => "notified".into(), Err(error) => format!("notify failed: {error}") }
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
                let app = app.clone();
                tokio::task::spawn_local(async move {
                    let service = hyper::service::service_fn(move |request| handle_request(request, app.clone()));
                    if let Err(error) = hyper::server::conn::http1::Builder::new().serve_connection(NwIo::new(connection), service).await {
                        eprintln!("  connection error: {error}");
                    }
                });
            }
        }).await
    })));
}
