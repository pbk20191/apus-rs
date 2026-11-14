//! rmcp (the official Rust MCP SDK) served on the main run loop with no thread owned by tokio.
//! - tokio current_thread runtime entered on the main thread; rmcp's service loop lives in a LocalSet polled by
//!   async-swift-rs `run_main`; a run loop timer turns tokio's timer wheel only while a request is in flight.
//! - The transport is ours (channels here; Network.framework later). The client is a plain thread standing in
//!   for a connection's callbacks.
use objc2_core_foundation::{kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ClientJsonRpcMessage, JsonRpcMessage, ServerCapabilities, ServerConfig, ServerJsonRpcMessage};
use rmcp::{schemars, tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

// ---- the guest runtime -------------------------------------------------------------------------------------
thread_local! {
    static RT: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) };
    static TICKER: RefCell<Option<objc2_core_foundation::CFRetained<CFRunLoopTimer>>> = const { RefCell::new(None) };
}
static TURNS: AtomicUsize = AtomicUsize::new(0);
const FAR: f64 = 1e10;
static KICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
thread_local! { static KICKER: RefCell<Option<objc2_core_foundation::CFRetained<CFRunLoopTimer>>> = const { RefCell::new(None) }; }
/// Polls the LocalSet, then asks for one turn of the runtime: whatever the poll handed to `tokio::spawn`
/// (rmcp sends responses from a JoinSet) runs right away instead of at the next tick.
struct KickAfterPoll<F>(F);
impl<F: std::future::Future + Unpin> std::future::Future for KickAfterPoll<F> {
    type Output = F::Output;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        let result = std::pin::Pin::new(&mut self.0).poll(cx);
        if KICK.load(SeqCst) { KICKER.with(|k| k.borrow().as_ref().unwrap().set_next_fire_date(CFAbsoluteTimeGetCurrent())) }
        result
    }
}
extern "C-unwind" fn turn(_timer: *mut CFRunLoopTimer, _info: *mut std::ffi::c_void) {
    TURNS.fetch_add(1, SeqCst);
    RT.with(|rt| rt.borrow().as_ref().unwrap().block_on(tokio::task::yield_now()));
}
fn ticking(on: bool) {
    TICKER.with(|t| t.borrow().as_ref().unwrap().set_next_fire_date(if on { CFAbsoluteTimeGetCurrent() } else { FAR }));
}
fn on_main() -> bool { async_swift_rs::is_main_thread() }

// ---- the transport -----------------------------------------------------------------------------------------
/// Messages in from any thread, messages out to any thread. Arms the tick while requests are unanswered.
struct ChannelTransport {
    incoming: tokio::sync::mpsc::UnboundedReceiver<ClientJsonRpcMessage>,
    outgoing: std::sync::mpsc::Sender<ServerJsonRpcMessage>,
    in_flight: Rc<Cell<usize>>,
}
// `Transport: Send` is required even with the `local` feature; this transport never leaves the main thread.
unsafe impl Send for ChannelTransport {}
impl rmcp::transport::Transport<RoleServer> for ChannelTransport {
    type Error = std::io::Error;
    fn send(&mut self, item: ServerJsonRpcMessage) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        if matches!(item, JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_)) {
            self.in_flight.set(self.in_flight.get().saturating_sub(1));
            if self.in_flight.get() == 0 { ticking(false) }
        }
        let sent = self.outgoing.send(item).map_err(|_| std::io::Error::other("peer gone"));
        std::future::ready(sent)
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        let Some(message) = self.incoming.recv().await else {
            // The peer is gone. rmcp's shutdown waits on tokio timers (a drain timeout), so the wheel must turn.
            ticking(true);
            return None;
        };
        if matches!(message, JsonRpcMessage::Request(_)) {
            self.in_flight.set(self.in_flight.get() + 1);
            ticking(true);
        }
        Some(message)
    }
    async fn close(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

// ---- the server --------------------------------------------------------------------------------------------
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SumRequest { a: i64, b: i64 }
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct WaitRequest { millis: u64 }

/// Not Send: holds main-thread state, the way a handler holding AppKit or accessibility objects would.
struct Desktop { tool_router: ToolRouter<Self>, calls: Rc<Cell<u32>> }

#[tool_router]
impl Desktop {
    #[tool(description = "Adds two numbers")]
    fn sum(&self, Parameters(SumRequest { a, b }): Parameters<SumRequest>) -> String {
        self.calls.set(self.calls.get() + 1);
        format!("{} (handler on main: {})", a + b, on_main())
    }
    #[tool(description = "Waits on a tokio timer, then reports")]
    async fn wait(&self, Parameters(WaitRequest { millis }): Parameters<WaitRequest>) -> String {
        self.calls.set(self.calls.get() + 1);
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(millis)).await;
        format!("asked {millis} ms, took {} ms (on main: {})", start.elapsed().as_millis(), on_main())
    }
    #[tool(description = "Does heavy work on a dispatch queue, off the main thread")]
    async fn heavy(&self) -> String {
        self.calls.set(self.calls.get() + 1);
        let worker_on_main = async_swift_rs::dispatch::spawn_detached(async { std::thread::sleep(Duration::from_millis(40)); on_main() }).await.unwrap();
        format!("worker on main: {worker_on_main}; back on main: {}; call #{}", on_main(), self.calls.get())
    }
}
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Desktop {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions("guest-mode spike")
    }
}

// ---- the client (a thread, standing in for a network connection) -------------------------------------------
fn client(to_server: tokio::sync::mpsc::UnboundedSender<ClientJsonRpcMessage>, from_server: std::sync::mpsc::Receiver<ServerJsonRpcMessage>) {
    let send = |json: serde_json::Value| to_server.send(serde_json::from_value(json).expect("valid client message")).unwrap();
    let reply = || serde_json::to_value(from_server.recv_timeout(Duration::from_secs(5)).expect("a reply within 5 s")).unwrap();
    send(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"spike","version":"0"}}}));
    let init = reply();
    println!("initialize → protocol {}, server {}", init["result"]["protocolVersion"], init["result"]["serverInfo"]["name"]);
    send(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    send(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    let tools = reply();
    println!("tools/list → {:?}", tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect::<Vec<_>>());
    let call = |id: u32, name: &str, args: serde_json::Value| {
        send(serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}));
    };
    let text = |v: serde_json::Value| format!("id {} → {}", v["id"], v["result"]["content"][0]["text"].as_str().unwrap_or(&v.to_string()));
    call(3, "sum", serde_json::json!({"a": 40, "b": 2}));
    println!("tools/call sum   {}", text(reply()));
    call(4, "wait", serde_json::json!({"millis": 120}));
    println!("tools/call wait  {}", text(reply()));
    call(5, "heavy", serde_json::json!({}));
    println!("tools/call heavy {}", text(reply()));
    // two at once: answers come back in completion order
    call(6, "wait", serde_json::json!({"millis": 200}));
    call(7, "sum", serde_json::json!({"a": 1, "b": 1}));
    println!("concurrent       {}", text(reply()));
    println!("concurrent       {}", text(reply()));
    let mut samples = vec![];
    for i in 0..40 {
        let sent = Instant::now();
        call(100 + i, "sum", serde_json::json!({"a": i, "b": 1}));
        reply();
        samples.push(sent.elapsed().as_micros());
        std::thread::sleep(Duration::from_millis(7));
    }
    samples.sort();
    println!("round trip of 40 sum calls: median {} µs, max {} µs   (kick after poll: {})", samples[20], samples[39], KICK.load(SeqCst));
    call(8, "nope", serde_json::json!({}));
    let error = reply();
    println!("unknown tool     id {} → error {}", error["id"], error["error"]["message"]);
    let turns = TURNS.load(SeqCst);
    std::thread::sleep(Duration::from_millis(600));
    println!("idle 600 ms: {} timer-wheel turns ({} during the whole session before that)", TURNS.load(SeqCst) - turns, turns);
    // dropping `to_server` closes the transport
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let handle = rt.handle().clone();
    RT.with(|slot| *slot.borrow_mut() = Some(rt));
    let timer = unsafe { CFRunLoopTimer::new(None, FAR, 0.05, 0, 0, Some(turn), std::ptr::null_mut()) }.unwrap();
    CFRunLoop::main().unwrap().add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes });
    TICKER.with(|t| *t.borrow_mut() = Some(timer));
    let kicker = unsafe { CFRunLoopTimer::new(None, FAR, FAR, 0, 0, Some(turn), std::ptr::null_mut()) }.unwrap();
    CFRunLoop::main().unwrap().add_timer(Some(&kicker), unsafe { kCFRunLoopCommonModes });
    KICKER.with(|k| *k.borrow_mut() = Some(kicker));
    KICK.store(std::env::args().nth(1).as_deref() != Some("nokick"), SeqCst);

    let (to_server, incoming) = tokio::sync::mpsc::unbounded_channel();
    let (outgoing, from_server) = std::sync::mpsc::channel();
    let threads_before = thread_count();
    std::thread::Builder::new().name("client".into()).spawn(move || client(to_server, from_server)).unwrap();

    let _guard = handle.enter();
    let local = tokio::task::LocalSet::new();
    let reason = async_swift_rs::run_main(KickAfterPoll(Box::pin(async move {
        local.run_until(async move {
            let server = Desktop { tool_router: Desktop::tool_router(), calls: Rc::new(Cell::new(0)) };
            let transport = ChannelTransport { incoming, outgoing, in_flight: Rc::new(Cell::new(0)) };
            let running = server.serve(transport).await.expect("initialize");
            let reason = running.waiting().await;
            ticking(false);
            format!("{reason:?}")
        }).await
    })));
    println!("service ended: {reason}; threads {} → {} (the client thread has exited; dispatch workers may remain)", threads_before, thread_count());
}

fn thread_count() -> u32 {
    extern "C" { fn task_threads(task: u32, list: *mut *mut u32, count: *mut u32) -> i32; static mach_task_self_: u32; }
    let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
    unsafe { task_threads(mach_task_self_, &mut list, &mut count) };
    count
}
