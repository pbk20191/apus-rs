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
