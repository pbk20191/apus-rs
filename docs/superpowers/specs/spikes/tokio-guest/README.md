# 스파이크: tokio를 메인 run loop의 손님으로 돌리기

tokio에는 공식 guest mode가 없다. `current_thread` 런타임으로 두 가지 방식을 실험했다.
실행: `cargo run --bin polling`, `cargo run --bin localset` (tokio 1.x, macOS arm64).

## polling: run loop 타이머가 `rt.block_on(yield_now())`를 호출

한 번 호출하면 준비된 태스크를 돌리고 만료된 타이머를 처리한 뒤 바로 돌아온다. `tokio::spawn`, `sleep`, `timeout`,
`select!`, 다른 스레드에서 오는 wake가 모두 메인 스레드에서 동작했다. 단점: tokio가 다음 타이머 시각이나 wake를
밖에 알려 주지 않아서 주기적으로 돌려야 한다 (5 ms 주기로 126 ms 동안 26번).

## localset: LocalSet은 메인에서, 드라이버는 보조 스레드에서

- 메인 스레드: `handle.enter()` 뒤 `run_main(local.run_until(work))`. LocalSet은 일반 Future라서 어떤 executor에서도 poll된다.
  `spawn_local` 태스크가 깨어나면 LocalSet의 waker(= run loop source signal)가 불린다.
- 보조 스레드: `rt.block_on(pending())`. 타이머 휠을 정확한 만료 시각으로 잠재우는 역할만 한다.
- 결과: `spawn_local` + 타이머 + 채널 + select 모두 메인 스레드에서 실행. 500 ms 유휴 구간 동안 LocalSet poll은 1번 (폴링 없음).
- 주의: 일반 `tokio::spawn`은 보조 스레드에서 실행된다. rmcp는 `local` feature를 켜면 서비스 루프가 `spawn_local`을 쓰지만
  `tool.rs`의 list-changed 알림과 `task_manager.rs`에는 `tokio::spawn`이 남아 있다 (rmcp 3.10.0 기준).

## nothread: tokio가 쓰는 스레드 0개

- 메인 스레드: LocalSet을 `run_main`으로 poll (wake는 이벤트 기반). 다른 스레드에서 온 wake는 tick 없이 35 ms에 도착했다.
- 타이머 휠: run loop 타이머(50 ms, tolerance 20 ms)가 `rt.block_on(yield_now())`로 돌린다. 이 타이머는 작업이 진행 중일 때만 켠다
  (`ticking(true/false)`를 embedder가 호출). 500 ms 유휴 구간 동안 turn 0번.
- 일반 `tokio::spawn`도 이 turn에서 메인 스레드로 실행된다.
- 스레드 수는 전후 동일 (tokio 스레드 없음).
- 한계: tokio 타이머의 정밀도가 tick 주기로 떨어진다 (`sleep(100ms)` → 116 ms). `tokio::spawn` 태스크는 타이머가 발화한 turn의
  다음 turn에 실행되어 한 tick 더 늦는다 (`sleep(10ms)` → 116 ms). turn을 "더 할 일이 없을 때까지" 반복하면 줄일 수 있다.
- tokio가 다음 만료 시각을 공개하지 않는 한 tick 없이 정확하게 만들 방법은 없다.

## rmcp_guest: rmcp 3.10.0을 nothread 구성 위에서 실행

`cargo run --bin rmcp_guest` (`nokick` 인자를 주면 poll 뒤 추가 turn을 끈다).

- rmcp는 `default-features = false, features = ["server", "macros", "local"]`. 핸들러(`Desktop`)는 `Rc<Cell<_>>`를 가진 `!Send` 타입이다.
- 트랜스포트는 채널로 만든 `Transport<RoleServer>` 구현이고, 클라이언트는 연결 콜백을 흉내 내는 일반 스레드다.
- 통과한 것: initialize, tools/list, tools/call (동기 도구, tokio 타이머를 기다리는 도구, dispatch 큐에서 일하고 돌아오는 도구),
  동시 요청 2개(완료 순서대로 응답), 없는 도구의 오류 응답, 클라이언트가 끊겼을 때 `QuitReason::Closed`로 종료.
- 모든 핸들러는 메인 스레드에서 실행. 동기 도구 왕복 시간은 중앙값 약 0.5 ms. 유휴 600 ms 동안 turn 0~2번.

알게 된 것:
- `Transport: Send`는 `local` feature에서도 요구된다. 메인 스레드 전용 상태를 가진 트랜스포트는 `unsafe impl Send`가 필요했다.
- rmcp는 응답 전송을 `JoinSet::spawn`(= `tokio::spawn`)으로 한다. 이 태스크는 LocalSet이 아니라 런타임 turn에서만 실행된다.
  그래서 LocalSet을 poll한 직후 turn을 한 번 더 돌리는 `KickAfterPoll`을 넣었다. 없으면 비동기 도구의 응답이 다음 tick까지 밀린다.
- 종료 처리도 turn이 필요하다. 연결이 끊긴 뒤 tick을 켜지 않았더니 서비스가 끝나지 않았다 (`receive`가 `None`을 돌려줄 때 tick을 켜서 해결).
  "tick 켜기를 빠뜨리면 멈춘다"는 위험이 실제로 나타난 사례다.
- `dispatch::spawn`은 메인 큐 태스크 안에서 부르면 메인 큐에 남는다. 메인 밖에서 일하려면 `spawn_detached`를 써야 한다.
- tokio 타이머 정밀도는 tick(50 ms)에 묶인다 (120 ms 요청 → 약 150 ms).

## nw_mcp: Network.framework + rmcp, HTTP로 MCP 서버

`cargo build --bin nw_mcp && ./test_nw.sh` (curl과 Python 클라이언트로 구동).

- 트랜스포트: Network.framework C API (`nw_listener`, `nw_connection`)를 직접 선언해서 사용, 블록은 `block2`. 모든 콜백을 메인 큐로 받는다.
  읽기/쓰기 완료 블록을 `oneshot`으로 future에 연결했다.
- HTTP/1.1: `httparse` + Content-Length만 처리 (keep-alive 포함). POST 하나에 JSON-RPC 메시지 하나, 응답은 `application/json`.
- 세션: rmcp 서비스 하나 = 세션 하나. 응답은 요청 id로 대기 중인 HTTP 요청에 돌려준다. GET 스트림이 없어서 서버가 먼저 보내는 메시지는 버린다.
- turn: poll 직후 1회(KICK), 요청 처리 중 50 ms(FAST), 세션이 살아 있는 동안 1 s(SLOW).

결과 (macOS 27.3 arm64, debug 빌드):
- initialize, notifications/initialized(202), tools/list, tools/call 3종, 없는 도구 오류, 잘못된 JSON(400), 세션 없는 요청(400), 동시 요청, DELETE로 세션 종료 모두 동작.
- 한 연결에서 `sum` 200회: 중앙값 152 µs, p95 203 µs.
- 프로세스 스레드 2개: 메인 스레드와 유휴 dispatch 워커 1개. tokio 스레드 없음.
- 유휴 2초 동안 turn: 세션이 있으면 약 2번/초(SLOW + stats 요청), 세션 종료 후에는 stats 요청에 따른 것뿐.

주의: `block2`는 C `bool` 블록 인자를 직접 받지 못해 `u8`로 선언했다.

## nw_hyper: 같은 서버, HTTP는 hyper

`cargo build --bin nw_hyper && BIN=nw_hyper ./test_nw.sh`

- `NwIo`가 `nw_connection`을 `hyper::rt::Read`/`Write`로 감싼다. hyper는 `http1` + `server` feature만 쓰고 타이머와 executor는 넘기지 않았다.
- 읽기 큐는 `dispatch_data` 그대로다: 새 조각은 `dispatch_data_create_concat`으로 붙이고, `dispatch_data_apply`가 주는 각 조각의 주소에서
  hyper 버퍼로 바로 복사하고, 읽은 만큼 `dispatch_data_create_subrange`로 잘라 낸다. 복사는 hyper 버퍼로 가는 1번뿐
  (`/stats`의 `bytes_copied_in`이 받은 바이트 수와 같다).
- 쓰기는 `dispatch_data_create`가 복사한다 (hyper는 바이트를 빌려줄 뿐). 미전송 바이트가 256 KB를 넘으면 `Pending`으로 역압을 건다.
- GET /mcp는 SSE 스트림 (`http_body::Body`를 채널로 구현). 서버가 먼저 보내는 메시지가 여기로 나간다:
  `announce` 도구가 보낸 `notifications/resources/list_changed`를 curl이 받았다.
- 300 KB 요청 본문(여러 조각으로 도착)도 정상 처리.

결과: nw_mcp의 모든 시나리오 통과. 한 연결에서 `sum` 200회: 중앙값 171 µs, p95 239 µs (직접 쓴 HTTP는 156 µs / 191 µs). 스레드 2개.

주의: 함수 이름은 `dispatch_data_create_subrange`다 (`subregion`은 없다).

## schema: rmcp 매크로가 만든 스키마 vs desktop-mcp의 `spec/tools.json`

`cargo run --bin schema` (Click, Scroll 두 도구로 비교).

- `#[tool(name, description, annotations(...))]`: annotations는 계약과 같게 나온다.
- `schemars`가 유도한 `inputSchema`는 의미는 같지만 형태가 다르다: `$schema`, `$defs`/`$ref`(enum), `"format": "int64"`,
  nullable이 `anyOf` 대신 `"type": ["array", "null"]`, `$ref`로 빠진 enum의 `default` 누락.
- 매크로에는 `input_schema = <expr>` 인자가 있어서, 라우팅과 인자 타입은 매크로로 하고 스키마만 계약에서 가져올 수 있다.

## stateless: 요청마다 rmcp 서비스 하나 (`serve_directly`)

`cargo run --bin stateless`. initialize 핸드셰이크 없이 `rmcp::service::serve_directly`로 서비스를 만들고, 메시지 하나를
전달한 뒤 응답이 나가면 닫는 `OneShot` 트랜스포트를 붙였다. `initialize`, `tools/list`, `tools/call`, `ping` 모두 응답했고
서비스는 매번 `Closed`로 바로 끝났다 (4개 요청에 약 5 ms). desktop-mcp의 기본 모드(stateless `POST /mcp`)를 이렇게 구현할 수 있다.
