# M0 보고서: Swift async 클로저를 같은 Task에서 await하기

작성일: 2026-11-13 · 실험 코드: `docs/superpowers/specs/spikes/m0/` (`./build.sh && ./m0 --m0`)
환경: macOS 27.3 arm64, Swift 6.4.2 런타임, rustc 1.99.1, Apple clang 21

## 판정: **PASS (조건부)**

spec §7.6의 기준을 모두 만족한다: 공개 Swift 기능만 쓰고, `@_silgen_name`이 없고, 레시피가 9줄이다.
다만 실험 중에 **spec의 "자식 Task 판단 규칙"이 `select`에서 동작할 수 없다는 것**이 드러났다.
이건 사용자 결정이 필요하다 (아래 "결정 필요").

## 결과 (30/30 반복 실행 모두 정상 종료)

| # | 시나리오 | 결과 |
|---|---|---|
| future | 기존 6개 시나리오를 future Task(결과 타입 `()`)로 | 6개 모두 통과, 30/30 |
| M0-1 | 클로저 안의 `withUnsafeCurrentTask` == 호출한 Rust 태스크 | 같음 |
| M0-2 | MainActor 태스크에서 `Task.sleep(50ms)` 클로저 | `"slept"`, 약 55ms. 호출 뒤에도 메인 스레드와 MainActor 유지 |
| M0-3 | throw하는 클로저 | `ProbeError.io(code: 5)`가 `any Error`로 전달됨 |
| M0-4 | Void 클로저 | `()` |
| M0-5 | 연속 3회 호출 | 모두 같은 Task |
| M0-6 | 호출 중 바깥 Task 취소 | 클로저가 `CancellationError`를 던짐 |
| M0-7 | `@TaskLocal` 아래 만든 Utility 태스크의 클로저 | `"ctx-local/0x11"`: task-local과 우선순위가 보임 |
| M0-8 | Swift Task 밖에서 await | `Err(NoCurrentTask)` |
| M0-9 | future Task 완료 | 정상 |
| C1 | `join!(호출, 호출)` | 두 호출 모두 자식 Task로 동시 실행. 50ms 두 개가 약 53ms. 자식 Task id ≠ 부모 |
| C2 | `select!(호출.child() 10초, timeout 30ms)` | 약 34ms에 timeout이 이김. 버려진 자식은 `CancellationError`로 정리됨 |
| C3 | `select!(호출 200ms, timeout 30ms)`, `.child()` 없음 | **약 203ms**: 같은 Task에서 실행되어 timeout이 무시됨 (아래 결정 필요) |
| C4 | 자식 Task의 task-local과 우선순위 | 부모 것이 보임 (`"ctx-local/0x11"` ×2) |
| C5 | 부모 future는 끝났지만 취소를 무시하는 자식이 남음 | 부모 Task와 `JoinHandle`은 자식이 끝난 뒤에 완료됨 (약 155ms) |
| Instruments | `xctrace` Swift Concurrency 템플릿 | Rust 태스크가 Swift Task로 기록됨 (수명 21건, 생성 21건, 취소 이벤트 3건). 이름은 `rs_task_create (N)` |

## 확정한 설계 결정 (Task 4, 8, 9가 따른다)

### 클로저 표현과 사용자 쪽 레시피 (9줄)

```swift
public final class AsyncSwiftClosure {
    let body: () async throws -> Any
    init(_ body: @escaping () async throws -> Any) { self.body = body }
}
/// Hands a Swift async closure to Rust as an owned (+1) pointer.
public func asyncSwiftClosure(_ body: @escaping () async throws -> Any) -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(AsyncSwiftClosure(body)).toOpaque()
}
```
사용 예: `@_cdecl("make_x") public func makeX() -> UnsafeMutableRawPointer { asyncSwiftClosure { try await x() } }`

- Rust는 `AsyncSwiftClosure` 객체 포인터(+1)를 받는다. 레이아웃: HeapObject 헤더 16바이트, 그다음 `body`의
  함수 word(+16), 컨텍스트 word(+24). 사용자 모듈의 non-resilient final class라 레이아웃이 고정된다.
  Rust는 다 쓰면 `swift_release`한다.
- 함수 word는 **`AsyncFunctionPointer`** 레코드를 가리킨다: `{ int32 상대 오프셋(코드), uint32 callee 컨텍스트 크기 }`.
  코드 주소는 `&rec.function + rec.function`이다 (arm64e가 아니라 서명 없음).

### 호출 규약

- 호출: `fn(indirect_any_result /*x0*/, callee_ctx /*swift_async_context*/, closure_ctx /*swift_context, +0*/)`.
  `callee_ctx = swift_task_alloc(rec.expected_context_size)`이고, Parent=호출한 쪽 ctx, ResumeParent=재개 함수.
- 반환: callee가 `ResumeParent(callee_ctx, error /*swift_context*/)`를 호출한다. `Any` 결과는 indirect 버퍼(32바이트)에 쓰인다.
  에러는 소유권 있는 `any Error`이고 `swift_errorRelease`로 해제한다.
- 재개 함수: `swift_task_dealloc(callee_ctx)`를 하고 Rust에 결과를 알린 뒤 poll을 이어 간다.
- **새 규칙 (spec에 없던 것)**: 호출이 끝난 뒤 MainActor 태스크는 반드시
  `swift_task_switch(ctx, step, main_executor)`로 원래 executor에 돌아가야 한다. 컴파일러가 nonisolated async 호출 뒤에
  넣는 것과 같은 동작이다. 빠뜨리면 메인이 아닌 스레드에서 재개되어 `assumeIsolated`가 trap한다 (실제로 재현됨).

### future Task

- `()`의 metadata는 **`$sytN` 심볼을 직접 쓰면 안 된다.** 이 심볼은 prefix를 포함한 FullMetadata의 시작을 가리키고,
  IRGen은 여기에 address point offset을 더해서 쓴다. 직접 넘기면 `swift_task_create_commonImpl`에서 크래시가 난다 (재현됨).
  대신 `swift_getTupleTypeMetadata(0, 0, NULL, NULL, NULL).metadata`를 쓴다.
- 진입 함수는 `AsyncGenericClosureEntryPoint(indirect_result, ctx, closure)` 모양이다.

### 자식 Task (Swift의 `async let`, discarding TaskGroup과 같은 구조)

- `IsChildTask` 플래그만으로는 부모의 상태 레코드에 등록되지 않는다 (취소 전파 없음). 그래서 **discarding TaskGroup**을 쓴다.
  - 첫 자식을 만들 때 `swift_taskGroup_initializeWithFlags(1<<8 /*DiscardResults*/, group, ()metadata)`.
    group 저장소(32 word)는 Task 초기 컨텍스트 안에 둔다.
  - 자식 생성 플래그: `IsChildTask(8) | EnqueueJob(12) | AddPendingGroupTaskUnconditionally(13) | IsDiscardingTask(14) | IsTaskFunctionConsumed(15)`.
    옵션 레코드는 `TaskGroup`(kind 1)이다. 자식 진입 함수는 `AsyncVoidClosureEntryPoint`이다 (discarding이면 non_future_adapter 경유).
  - 부모 완료 직전에 group이 있으면 `swift_taskGroup_waitAll(result, ctx, group, NULL, after_wait_all, &wait_ctx)`를 호출한다.
    `wait_ctx`는 `TaskFutureWaitAsyncContext`(16바이트 + 3 word)다. 재개되면 `swift_taskGroup_destroy` 후 완료한다.
- **`JoinHandle`은 Swift Task가 완료된 시점(`rs_finish`)에 결과를 낸다.** Rust future가 끝난 시점이 아니다.
  그래야 Swift의 `Task.value`처럼 자식이 끝난 뒤에 결과가 나온다 (C5).
- 버려진 호출(`SwiftAsync` drop)은 자식이면 `swift_task_cancel`하고, 결과 버퍼는 호출이 끝날 때까지 공유 레코드가 살려 둔다.

## 결정 필요: 언제 자식 Task로 실행하나

spec은 "대기 중인 호출이 정확히 하나이고 다른 깨우기가 없으면 같은 Task, 아니면 자식"이라고 했다.
그런데 `select!(호출, sleep(30ms))`에서는 poll 시점에 호출이 하나이고 아직 아무것도 깨어나지 않았다.
"나중에 깨어날 예정인 다른 future"는 알 방법이 없다. 그래서 같은 Task에서 실행되고 timeout이 무시된다 (C3).

실험에서 동작한 규칙은 다음과 같다.
- 한 poll에 호출이 2개 이상 등록되면(`join!`) 자동으로 자식 Task로 실행한다. 또는 poll 중에 깨우기가 있었으면 자식으로 실행한다.
- `.child()`를 붙이면 항상 자식 Task로 실행한다.
- 그 외 단독 await는 같은 Task에서 실행한다. **Swift의 `await f()`와 같은 의미다.** Swift에서도 동시 실행은
  `async let`이나 TaskGroup으로 명시해야 한다.

**추천**: 이 규칙을 채택한다. `select`나 timeout과 함께 쓸 때는 `.child()`(또는 이를 감싼 `swift::timeout(d, call)` 헬퍼)를
쓰도록 문서화하고, spec의 시나리오 10은 `.child()`를 쓰는 형태로 바꾼다.
**대안**: 모든 호출을 항상 자식 Task로 실행한다. `select`가 언제나 기대대로 동작하지만, "같은 Task에서 실행"(G3)이 사라진다.

## 기타 관찰

- 취소를 무시하는 자식(`usleep(80ms)`)이 약 155ms 걸렸다. 동작은 맞지만, cooperative pool에서 블로킹이
  지연을 만든 것으로 보인다. README에 "블로킹 금지"로 남긴다.
- Instruments 이름을 더 알아보기 쉽게 하려면 `InitialTaskName` 옵션 레코드(kind 7)로 Task 이름을 줄 수 있다. 다음 단계 후보다.
