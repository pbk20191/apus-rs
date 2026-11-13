# async-swift-rs 설계

작성일: 2026-11-13 · 상태: 승인됨. M0 결과 반영 (`m0-report.md`)

## 1. 의도

Rust의 `async`/`await`를 **Apple의 Swift Concurrency 런타임 위에서** 실행하는 크레이트다.
Rust 태스크가 Swift 런타임이 보기에 **진짜 Swift Task**가 되어, Swift 쪽 기능들
(취소, 우선순위와 우선순위 상승, `@TaskLocal`, MainActor 격리, 그 안에서 만든 `Task {}`의 상속)이
Rust 태스크에도 그대로 적용되게 하는 것이 핵심이다.

같은 API로 **libdispatch(GCD) 백엔드**도 제공한다. Swift 런타임에 의존하지 않는 가벼운 대안이다.

### 왜

최종 목표는 "Rust가 메인이고 SwiftUI, StoreKit 2, SwiftData를 Swift 경계 모듈로 붙이는 macOS 앱"이다.
그런 앱에서 tokio 같은 별도 런타임을 돌리면 스레드 풀이 두 개가 되고, 취소, 우선순위, 메인 스레드 규칙이
Swift 쪽과 따로 놀게 된다. 이 크레이트는 그 앱의 런타임이 될 후보이고, **지금은 독립된 실험 크레이트**로 만든다.

## 2. 목표, 비목표, 반목표

**목표**
- G1. `spawn`한 Rust future가 `swift_task_create_common`으로 만든 **진짜 Swift Task**로 실행된다.
- G2. 위 Task 안에서 Swift 코드가 보는 상태가 일관된다: `withUnsafeCurrentTask != nil`, `Task.isCancelled`,
  `Task.currentPriority`, 우선순위 상승, task-local 복사, MainActor 격리(`MainActor.assumeIsolated` 통과).
- G3. Rust 태스크 안에서 **Swift async 클로저를 같은 Task 안에서 직접 await**할 수 있다 (첫 버전부터 포함).
- G4. 같은 공통 API를 가진 **libdispatch 백엔드**(QoS, 메인 큐, serial queue, `block_on`).
- G5. 크레이트 자체에는 **Swift 소스가 없다.** 호출 규약은 C shim의 clang `swiftcall`/`swiftasynccall`로 맞춘다.

**비목표 (첫 버전)**
- Rust에서 Swift async 함수에 **인자를 직접 전달**하기. 인자는 Swift 쪽에서 클로저를 만들 때 캡처한다.
- **반대 방향**, 즉 Swift 코드가 Rust 태스크의 결과를 await하거나 취소하기
  (예: SwiftUI `.task { await rustThing() }`). 다음 버전 기능이다. 다만 첫 버전 설계가 이 기능을
  막지 않도록 Task를 결과값을 갖는 future Task로 만든다 (7.1). *(제 판단: 사용자가 범위 질문에 답하지 않고 "진행"이라고 해서, 설계만 열어 두는 중간안을 택함)*
- 결과 타입별 전용 ABI 경로. ABI는 `() async throws -> Any` 한 가지 모양만 구현한다.
- 비동기 I/O reactor(네트워크, 파일). 실행기(executor)만 제공하고, I/O는 `async-io` 같은 executor 독립 크레이트에 맡긴다.
- Rust에서 Swift 타입이나 conformance 정의, task group, `async let` 같은 구조적 동시성.
- iOS, Linux, Windows 지원. (iOS는 나중 후보)
- tokio 호환 레이어.

**반목표 (이렇게 되면 기술적으로 동작해도 실패)**
- 크레이트에 Swift 소스나 swiftc 빌드 단계가 들어가는 것. (테스트 전용 Swift probe는 예외)
- arm64 전용 인라인 어셈블리로 호출 규약을 손으로 맞추는 것. 호출 규약은 clang이 정해야 한다.
- Swift 쪽에서 Rust 태스크를 "가짜 Task"로 보는 것. 예: Swift API 호출을 `Task {}` + 콜백으로 감싸서
  취소나 우선순위가 끊기는 구조를 기본 경로로 삼는 것.
- ABI 가정이 틀렸을 때 조용히 메모리를 망가뜨리는 것. 시작 시 자체 점검이 실패하면 명확히 실패해야 한다.

## 3. 제약

- 플랫폼: macOS. 아키텍처는 arm64와 x86_64 둘 다 지원한다 (실험에서 C shim이 양쪽에서 동작함을 확인).
  arm64e 빌드는 범위 밖이다 (pointer authentication 서명이 필요해짐).
- Swift 런타임은 OS에 내장된 것을 쓴다 (`/usr/lib/swift`의 `libswiftCore`, `libswift_Concurrency`).
- C shim은 `cc` 크레이트로 clang 컴파일한다. clang의 `swiftcall`, `swiftasynccall`, `swift_context`,
  `swift_async_context` 속성이 필요하다 (Apple clang에 있음).
- ABI 정보의 출처는 `swiftlang/swift`의 **`swift-6.4.2-RELEASE` 태그**다 (실험 환경의 OS 런타임과 일치).
  옮긴 구조체, 상수, 플래그마다 출처 파일을 주석으로 남긴다.
- 라이선스: 사용자의 다른 프로젝트(desktop-mcp, MIT)와 함께 쓸 수 있어야 한다. GPL 코드는 복사 금지
  (예: `eugenehp/rswift`). rswift의 `swift-runtime-sys`는 사용자 요청으로 설계 수준에서 한 번 검토했다. 가져온 코드는 없고,
  Rust future를 Swift Task로 실행하는 기능은 그쪽에 없다. Apache-2.0인 `dgrijalva/async-dispatch`는 참고 가능하며, 코드를 가져오면 출처를 표기한다.

## 3.1 코드 배치 (사용자 결정, M0 이후)

- **Rust → 런타임 (동기 함수)**: C를 거치지 않고 Rust에서 `extern "C"`로 직접 호출한다. 그러면 글루가 Rust 안에서 인라이닝된다.
  대상은 Task 생성, continuation resume과 init, cancel, escalate, 현재 Task 조회, `swift_task_alloc`/`dealloc`,
  TaskGroup 생성과 해제, metadata 조회, Job enqueue다.
  근거: clang이 이 16개 함수에 대해 만드는 `swiftcall` 호출 코드와 C 호출 코드가 arm64와 x86_64에서 같다.
  이 사실은 테스트로 고정한다.
  예외: Swift 규약의 `bool` 반환은 하위 1비트만 정의되므로 `u8`로 받아 `& 1`로 거른다.
- **런타임 → 우리 코드 (async funclet, `swift_context` 인자)**: C에 남긴다. 진입 함수, step, 재개 지점, 호출 후 복귀,
  자식 진입, `waitAll` 후 복귀, 취소 핸들러, Job invoke가 여기에 속한다. 런타임이 함수 포인터로 부르므로 어떤 방식이든 인라이닝 대상이 아니다.
- C funclet → Rust `rs_poll` 호출(poll마다 1회)의 인라이닝은 언어 경계 LTO가 필요해 범위 밖이다. 측정값: 경계 호출 비용은 몇 ns,
  poll 한 사이클은 50~130ns.
- 대안이었던 Swift 트램펄린(공개 Swift API로 Task 본문 작성)은 채택하지 않았다. Rust↔Swift 경계가 LTO로 막혀 있어 글루를 인라이닝할 수 없기 때문이다.

## 4. 이름과 구성

- 저장소, 디렉터리: `async-swift-rs`
- 패키지 이름: `async-swift-rs` (사용자 결정). 라이브러리 경로는 `async_swift_rs`.
  *(제 판단, 확인 필요: crates.io 관례상 패키지명은 `async-swift`, 저장소명만 `-rs`로 두는 방법도 있다.)*
- 크레이트 하나에 백엔드 두 개를 feature로 둔다.
  - `swift` (기본): Swift Concurrency 백엔드. 최상위 모듈.
  - `dispatch`: libdispatch 백엔드. `async_swift_rs::dispatch` 모듈.
  - 두 feature를 동시에 켤 수 있다.

## 5. 공통 API (두 백엔드 동일)

API 모양은 `async-dispatch`를 기준으로 한다.

```rust
let h: JoinHandle<T> = spawn(fut);                        // 기본 우선순위
let h = Builder::new().priority(Priority::Utility).spawn(fut);
let h = spawn_on_main(fut);                               // 메인 스레드 / MainActor
sleep(Duration).await;
h.await -> Result<T, JoinError>                           // JoinError::{Aborted, Panicked}
h.abort();                                                // 아래 7.4 참고
drop(h);                                                  // detach: 태스크는 계속 실행됨
```

- `Priority`는 QoS 값과 같은 체계를 쓴다: `UserInteractive`(0x21), `UserInitiated`(0x19), `Default`(0x15),
  `Utility`(0x11), `Background`(0x09), `Unspecified`(0). Swift의 `JobPriority`와 같은 값이다.
- future는 `Send + 'static`이어야 한다. *(제 판단)* `spawn_on_main`은 `!Send` future도 받는
  `spawn_local_on_main` 변형을 둔다 (메인 스레드에서만 호출 가능, 아니면 panic).
- 패닉: 태스크 안의 패닉은 잡아서 `JoinError::Panicked`로 전달한다. 패닉이 FFI 경계를 넘어가면 안 된다.

### 상속 규칙 *(제 판단: Swift `Task {}`와 같게)*

- `spawn`은 Swift의 `Task {}`와 같은 의미다.
  - 우선순위: `Unspecified`로 만들어 런타임이 현재 Task 또는 스레드 QoS에서 상속하게 한다.
  - task-local: 복사한다 (`CopyTaskLocals`).
  - actor: MainActor 태스크 안에서 `spawn`하면 MainActor에 남는다.
- `spawn_detached`는 Swift의 `Task.detached`와 같은 의미다. 상속하지 않고, 우선순위는 `Default`다.
- `Builder`로 우선순위, 메인 여부, task-local 복사를 하나씩 덮어쓸 수 있다.
- dispatch 백엔드에서는 task-local 개념이 없어 무시한다. 우선순위는 QoS 상속으로, MainActor 상속은
  "메인 큐 태스크 안에서 spawn하면 메인 큐"로 대응한다.

### 메인 스레드 런루프

`spawn_on_main` 태스크는 메인 큐가 처리돼야 실행된다. 실험에서 **`dispatch_main()`은 메인 큐 작업을 메인이 아닌
워커 스레드에서 실행**한다는 것을 확인했다. MainActor 격리는 유지되지만 AppKit이 요구하는 진짜 메인 스레드가 아니다.
그래서:

- 문서에 "메인 큐는 `CFRunLoopRun` 또는 `NSApplication.run`으로 처리해야 하며 `dispatch_main`을 쓰면 안 된다"고 명시한다.
- *(제 판단)* 편의 함수 `run_main(fut) -> T`를 제공한다: `fut`를 메인에서 spawn하고 `CFRunLoopRun`을 돌린 뒤,
  끝나면 런루프를 멈추고 결과를 반환한다. 메인 스레드에서 호출하지 않으면 panic.

## 6. libdispatch 백엔드

- 구조는 `async-dispatch`(Zed gpui dispatcher 계열)와 같다: `async-task`로 `Runnable`을 만들고,
  schedule 콜백에서 `dispatch_async_f(queue, runnable, trampoline)`을 호출한다.
- `async-dispatch`에 없는 것을 더한다.
  - **QoS**: `Priority` → `dispatch_get_global_queue(qos, 0)`.
  - **메인 큐**: `dispatch_get_main_queue()` (실제 심볼은 `_dispatch_main_q`).
  - **serial queue**: `SerialQueue::new(label, priority)`와 `queue.spawn(fut)`. 같은 큐의 태스크는 동시에 실행되지 않는다.
  - **`block_on(fut)`**: 현재 스레드를 park하고 waker로 unpark한다. 메인 스레드에서 메인 큐 태스크를 기다리면
    교착되므로, 그 경우는 panic하고 `run_main`을 쓰라고 안내한다.
  - **`sleep`**: `dispatch_after_f`.
- bindgen은 쓰지 않는다. 필요한 dispatch 함수 몇 개를 직접 `extern "C"`로 선언한다.
- 실험 결과(참고: `spikes/gcd/main.rs`): 50ms sleep 1만 개를 동시에 실행해 약 66ms, await 후에도 QoS 유지.

## 7. Swift Concurrency 백엔드

### 7.1 태스크 생성 (실험으로 확인됨, `spikes/swift-task/`)

- `swift_task_create_common(flags, options, result_type, entry, closure_context, sizeof(RsTaskContext))`.
  - 실험에서는 `result_type = NULL`(결과 없는 non-future Task)로 확인했다.
  - 첫 버전은 **future Task**로 만든다. `result_type`은 `()`의 metadata이고, `$sytN` 심볼을 직접 쓰지 않고
    `swift_getTupleTypeMetadata(0, 0, NULL, NULL, NULL)`로 얻는다 (심볼이 prefix 포함 위치를 가리켜 크래시가 났음, M0).
    진입 함수는 `AsyncGenericClosureEntryPoint`(indirect result, ctx, closure) 모양이며, 컨텍스트 크기는
    `FutureAsyncContext` 이상이어야 한다. 다음 버전에서 Swift가 이 Task를 await할 수 있게 하려는 준비다.
    M0에서 같이 확인하고, 안 되면 non-future로 돌아가되 그 사실을 보고한다.
  - `flags` = 우선순위(bit 0~7) | `EnqueueJob`(bit 12) | `IsTaskFunctionConsumed`(bit 15) | 필요 시 `CopyTaskLocals`(bit 10).
  - `closure_context`는 Rust 태스크 상태 포인터다. `IsTaskFunctionConsumed` 플래그를 주기 때문에 런타임이
    이걸 Swift 객체로 보고 `swift_release`하지 않는다.
  - MainActor 태스크는 `InitialSerialExecutor` 옵션 레코드(kind 0)에 `swift_task_getMainExecutor()`를 넣는다.
- 진입 함수는 `AsyncVoidClosureEntryPoint` 모양이다: `swiftasynccall void entry(swift_async_context ctx, swift_context closure)`.
- 초기 컨텍스트 레이아웃(우리 소유): `AsyncContext` 헤더(런타임이 Parent=NULL, ResumeParent=`completeTaskAndRelease`로 채움)
  뒤에 재사용할 `ContinuationAsyncContext`, Rust 상태 포인터, 취소 핸들러 레코드, 재개할 executor를 둔다.

### 7.2 poll과 중단, 재개

- `step(ctx)`: Rust future를 poll한다.
  - **Ready**: 취소 핸들러를 제거하고, Rust 상태를 정리하고, `ResumeParent(ctx, error=NULL)`를 tail call한다.
  - **Pending**: continuation 컨텍스트를 채운다 (Parent=ctx, ResumeParent=`resume_point`).
    `swift_continuation_init` → Rust 쪽에 continuation 등록 → `swift_continuation_await`를 tail call한다.
- Rust waker: 태스크가 park 상태면 `swift_continuation_resume(task)`, poll 중이면 "notified" 표시만 한다.
  poll 중에 깨어난 경우에는 park하지 않고 스스로 resume해서 즉시 다시 poll한다.
- MainActor 태스크는 continuation에 `HasExecutorOverride`(bit 1) 플래그와 메인 executor를 넣는다.
  **이걸 빠뜨리면 재개될 때 일반 스레드 풀로 빠진다.**
- tail call은 디스어셈블해서 `b`/`br`로 생성됐는지 확인한다 (clang이 swiftasynccall에서 보장).
- *(제 판단)* 상태 머신은 `async-task`의 `Runnable`/schedule 모델로 구현을 시도한다
  (schedule 콜백 = continuation resume). 맞지 않으면 실험의 직접 구현 상태 머신
  (IDLE/RUNNING/NOTIFIED/PARKED/DONE)으로 간다. 어느 쪽이든 race 테스트를 통과해야 한다.

### 7.3 타이머

- `swift_task_enqueueGlobalWithDelay`에 waker를 깨우는 **plain Job**을 넣는다 (실험: `spikes/swift-job/`).
- plain Job은 런타임 내부의 `jobHeapMetadata`가 export되지 않으므로, 같은 모양의 `DispatchClassMetadata`를
  직접 만든다 (kind `0x503`, `dispatchSwiftObjectType = 1`, invoke → `swift_job_run`).
  JobKind는 런타임에서 쓰지 않는 값 `1`을 쓴다.

### 7.4 취소와 abort

- 태스크 시작 시 `swift_task_addCancellationHandler`로 핸들러를 등록한다.
  핸들러 타입은 `SWIFT_CC(swift) void(SWIFT_CONTEXT void*)`라서 **인자가 x20(swift_context)으로 온다.**
- Swift 쪽 취소(Swift 코드가 `task.cancel()`을 부르든, Rust가 `swift_task_cancel`을 부르든)는 Swift 의미 그대로
  **협조적 취소**다: Rust 쪽 `cancelled().await`가 깨어나고 `is_cancelled()`가 true가 된다. future를 강제로 버리지 않는다.
- *(제 판단)* `JoinHandle`은 두 가지를 제공한다.
  - `cancel()`: 협조적 취소만 (Swift `Task.cancel()`과 동일).
  - `abort()`: 협조적 취소에 더해, 다음 poll 시점에 future를 drop하고 `JoinError::Aborted`로 끝낸다
    (`async-dispatch`, tokio와 같은 의미).

### 7.5 현재 태스크 API (Swift 백엔드 전용)

`current_task()`, `is_cancelled()`, `cancelled()`(future), `priority()`, `JoinHandle::escalate(p)`.
모두 `swift_task_getCurrent`, `swift_task_isCancelled`, `swift_task_currentPriority`, `swift_task_escalate`를 쓴다.

### 7.6 Swift async 클로저를 같은 Task에서 await하기 (G3)

**ABI 모양은 하나**: `() async throws -> Any`.
Swift 6에서 `() async throws -> T`, `() async -> T`, `() async throws`, `() async`, `() async -> T?`가 모두
어댑터 코드 없이 이 타입으로 암시적 변환되는 것을 확인했다. 변환 thunk는 Swift 컴파일러가 Swift 쪽에서 만든다.

- **인자 없음**: 인자는 사용자 앱의 Swift 모듈이 클로저를 만들 때 캡처한다.
  클로저를 만드는 동기 함수는 사용자 앱 쪽에 있으며 C ABI(`@_cdecl`)로 노출된다. 이 부분은 단순한 값 전달이다.
- 호출 흐름 (예상, **M0 실험으로 확정**):
  1. 클로저 값(async 함수 포인터와 컨텍스트, 2 word)을 받는다.
  2. `swift_task_alloc`으로 callee 컨텍스트를 할당하고, Parent와 ResumeParent를 채운 뒤 tail call한다.
  3. 결과 `Any`(existential, 32바이트)는 indirect result 버퍼로 받고, 에러는 재개 함수의 인자로 받는다.
  4. 재개 함수에서 callee 컨텍스트를 `swift_task_dealloc`하고 Rust future를 깨운다.
- **다른 Rust future와 섞일 때의 규칙 (사용자 결정, M0 이후 확정)**:
  - 단독으로 await하면 **같은 Task 안에서** 실행한다. Swift의 `await f()`처럼 그동안 Task를 점유하므로,
    같은 태스크의 다른 future는 Swift 호출이 끝날 때까지 poll되지 않는다.
  - 한 poll에 Swift 호출이 2개 이상 등록되거나(`join!`), poll 도중에 깨우기가 있었으면 그 호출들은 **자식 Task**로 실행한다.
  - `.child()`를 붙인 호출은 항상 자식 Task로 실행한다. `swift::timeout(d, call)` 헬퍼는 내부적으로 `.child()`를 쓴다.
  - "나중에 깨어날 예정인 다른 future"는 알 수 없다. 그래서 `select!`이나 timeout과 함께 쓸 때는 `.child()`가 필요하다.
    `.child()` 없이 쓰면 같은 Task에서 끝까지 실행된다 (M0 C3). 이 내용을 README에 분명히 쓴다.
  - 자식 Task는 **discarding TaskGroup**으로 만든다. `IsChildTask`만으로는 부모 상태 레코드에 등록되지 않기 때문이다
    (세부는 `m0-report.md`). 부모는 완료 직전에 `swift_taskGroup_waitAll`로 자식을 기다린다.
  - timeout은 협조적 취소로 동작한다: 시간이 되면 자식 Task를 취소한다.
  - **호출이 끝난 뒤 MainActor 태스크는 `swift_task_switch`로 메인 executor에 돌아간다** (컴파일러와 같은 동작).
  - **`JoinHandle`은 Swift Task가 완료된 시점에 결과를 낸다.** 자식이 남아 있으면 그 자식들이 끝난 뒤다 (Swift `Task.value`와 같음).
  - `abort()`는 같은 Task에서 실행 중인 Swift 호출이 반환될 때까지 기다린 뒤 적용된다. 그동안 협조적 취소는 즉시 전달된다.
- Rust 쪽 표현:
  - `SwiftAsync`: 소유권 있는 클로저 값. `.await` → `Result<SwiftAny, SwiftError>`.
  - `SwiftAny`: 소유권 있는 `Any`. `downcast::<T>()`는 `swift_dynamicCast`를 쓴다.
    첫 버전 지원 타입: `()`, `bool`, `i64`/`i32`/`u64`, `f64`, `String`, 그리고 `Option<_>`.
    그 외는 불투명 핸들로 남겨 다시 Swift로 넘길 수 있게 한다.
  - `SwiftError`: 소유권 있는 `any Error`. *(제 판단)* 메시지는 `NSError`로 캐스팅한 뒤
    CF 함수(`CFErrorCopyDescription`, toll-free bridging)로 얻는다. objc2 의존은 피한다.
- **사용자 쪽 Swift 레시피**: 클로저를 Rust로 넘기는 Swift 코드 조각은 README에 레시피로 제공한다
  (크레이트에 Swift 소스를 넣지 않는다). 정확한 형태는 M0에서 결정한다.
  클로저는 C로 표현할 수 없으므로 레시피가 어떤 식으로든 감싸야 한다.
  *(제 판단)* M0 통과 기준:
  - 공개 Swift 기능만 쓴다. 두 word짜리 thick function 레이아웃에 기대는 `unsafeBitCast`는 허용한다.
    Apple 플랫폼에서 ABI로 고정된 레이아웃이기 때문이다.
  - `@_silgen_name` 같은 언더스코어 속성은 쓰지 않는다.
  - 레시피는 사용자가 복사해 붙이는 **20줄 이내의 헬퍼 하나**여야 한다. 그보다 크면 8절 (a)의
    "Swift 트램펄린"으로 보고 멈춘 뒤 다시 논의한다.
- **Swift Task 밖에서 await한 경우** *(제 판단)*: dispatch 태스크 안, `block_on` 안, 일반 스레드처럼 현재 Swift Task가
  없는 곳에서 `SwiftAsync`를 await하면 `Err(SwiftError::NoCurrentTask)`를 반환한다.
  별도 Task를 몰래 만들어 대신 실행하지 않는다 (반목표의 "가짜 Task"). `SwiftAsync`는 `swift` feature 전용이다.

### 7.7 시작 시 자체 점검

첫 spawn 전에 한 번 확인한다: Task 하나를 만들어 poll, 중단, 재개, 완료가 정상인지, 그리고 그 안에서
`swift_task_getCurrent`가 null이 아닌지. 실패하면 명확한 에러로 panic한다. 조용히 계속 진행하지 않는다.
*(제 판단)* 자동으로 dispatch 백엔드로 폴백하지는 않는다. 의미가 달라지기 때문이다.

## 8. 위험

| 위험 | 내용 | 대응 |
|---|---|---|
| 비공식 ABI | `swift_task_create_common`은 export되지만 컴파일러가 쓰는 공개 진입점이 아니다. 컨텍스트 레이아웃과 플래그 비트, plain Job metadata 모양은 런타임 내부 계약이다 | 출처 주석, 7.7 자체 점검, macOS 베타마다 회귀 테스트 |
| JobKind 1 | 런타임이 나중에 이 값에 의미를 부여할 수 있다 | 자체 점검, 문서화 |
| 레이스 | 깨우기와 park가 겹치는 구간 | 상태 머신 단위 테스트, 반복 스트레스 테스트 |
| Swift async 호출 (G3) | 아직 실험하지 않았다 | M0에서 먼저 확인. 안 되면 범위를 다시 논의 (아래) |
| `dispatch_main` 함정 | 메인 큐가 메인 스레드가 아닌 곳에서 처리됨 | 문서화, `run_main` 제공 |

**G3가 M0에서 안 되면**: 지금 범위를 그대로 밀어붙이지 말고 사용자와 다시 논의한다.
대안은 (a) 작은 Swift 트램펄린 허용, (b) `Task {}` + 콜백 방식으로 첫 버전을 내고 G3를 다음 버전으로 미루는 것이다.

## 9. 테스트

- 메인 스레드와 런루프가 필요하므로 통합 테스트는 `harness = false` 바이너리로 둔다 (`libtest-mimic` 등).
  메인 스레드는 런루프를 돌리고 테스트 본문은 태스크로 실행한다.
- **Swift probe (테스트 전용)**: 테스트 빌드에서만 swiftc로 작은 Swift 파일을 컴파일해 링크한다.
  Swift 입장에서 현재 Task를 확인하는 함수들이다 (`withUnsafeCurrentTask`, `Task.isCancelled`, `Task.currentPriority`,
  `MainActor.assumeIsolated`, `@TaskLocal` 읽기, 안에서 만든 `Task {}`의 우선순위, 그리고 G3용 async 클로저).
  참고: `spikes/swift-task/probe.swift`.
- 반드시 통과해야 하는 시나리오 (실험에서 확인한 6개 + 추가):
  1. MainActor 태스크: Task 있음, 메인 스레드, `assumeIsolated` 통과
  2. await 후에도 1이 유지됨
  3. utility 태스크: Swift가 보는 우선순위 0x11, 안에서 만든 `Task {}`가 0x11을 상속
  4. `swift_task_cancel` → Rust `cancelled()`가 깨어나고 Swift `Task.isCancelled == true`
  5. 중단 중 우선순위 상승 → 재개 후 Swift가 보는 우선순위가 올라가 있음
  6. `@TaskLocal` `withValue` 안에서 spawn: `CopyTaskLocals`면 값이 보이고, 아니면 기본값
  7. (G3) Rust 태스크에서 Swift async 클로저를 await: 호출된 쪽의 `withUnsafeCurrentTask`가 같은 Task를 가리킴,
     결과와 에러 전달, 클로저 실행 중 취소하면 `Task.sleep`이 `CancellationError`를 던짐
  8. 스트레스: spawn, wake, cancel을 대량으로 반복해 크래시, 누수, 행이 없음
  9. abort: future가 drop되고 `JoinError::Aborted`
  10. (G3) `join!(swift_a, swift_b)`가 자식 Task로 동시에 실행되고, `select!(swift_call.child(), sleep)`의 timeout이
      자식 Task를 취소하며, 자식 안에서 부모의 task-local과 우선순위가 보이고, 부모의 `JoinHandle`은 자식이 끝난 뒤에 결과를 냄
- 아키텍처: arm64에서 전부, x86_64는 Rosetta로 최소한 1~6을 실행.

## 10. 단계

- **M0 (관문)**: G3 실험. Swift async 클로저를 같은 Task에서 await한다. 클로저 값의 표현과 사용자 쪽 Swift 레시피를 확정한다.
  자식 Task로 실행하는 경로(`IsChildTask`)와 future Task 생성(`$sytN` 결과 타입)도 함께 확인한다.
  추가로 Instruments의 Swift Tasks 뷰에 Rust 태스크가 보이는지 확인한다.
  결과를 사용자에게 보고하고, 8절 조건에 해당하면 여기서 멈춘다.
- M1: dispatch 백엔드와 공통 API, 테스트 하네스.
- M2: Swift 백엔드 코어 (7.1~7.5, 7.7).
- M3: G3 (7.6).
- M4: README(레시피, 런루프 주의사항, ABI 출처와 위험), 예제.

## 11. 구현자에게 맡기는 것

- 모듈 구조, 내부 타입 이름, 에러 타입 세부.
- `async-task` 사용 여부 (7.2의 판단 기준 안에서).
- 테스트 하네스 구현 방식.
- 최소 지원 macOS 버전: 실험 환경은 macOS 27.3, Swift 6.4.2다. 더 낮은 버전 지원은 확인할 수 있는 범위에서 정하고 문서화한다.

## 12. 참고 자료

- 실험 코드 (버리는 코드, 참고용): `docs/superpowers/specs/spikes/`
  - `gcd/`: std만으로 만든 libdispatch executor
  - `swift-job/`: plain Swift Job, MainActor 큐에 넣기, `dispatch_main` 함정 재현(`--dispatch-main`)
  - `swift-task/`: 진짜 Swift Task로 실행하는 실험 (시나리오 1~6)
- Swift 런타임 소스 (`swiftlang/swift` @ `swift-6.4.2-RELEASE`):
  `include/swift/ABI/Task.h`, `ABI/TaskStatus.h`, `ABI/MetadataValues.h`, `ABI/MetadataKind.def`, `ABI/Metadata.h`,
  `ABI/Executor.h`, `Runtime/Concurrency.h`, `stdlib/public/Concurrency/Task.cpp`
- `dgrijalva/async-dispatch` (Apache-2.0): dispatch 백엔드의 API와 구조 참고
