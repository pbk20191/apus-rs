# isolation과 actor: `#isolation` 함수, actor sync 호출, typed throws, `next(isolation:)`

작성일: 2026-11-17 · 상태: 설계 (소유자 승인: M5 실험 결과와 위험 표의 판단) · 실험: `spikes/m5-isolation/NOTES.md`

## 1. 의도

Rust에서 Swift 6 스타일 API를 그대로 부른다. 소유자가 든 대상:
- `isolation: isolated (any Actor)? = #isolation`을 받는 함수
- actor의 sync 메서드(바깥에서 부르려면 그 actor로 옮겨 가야 함)
- 대표 사례인 현대 async iterator `AsyncIteratorProtocol.next(isolation:)`(SE-0421, macOS 15)

소유자 결정 (2026-11-17)
- ABI로 보장하기 어려운 것은 진행 전에 알린다(아래 6절 표로 알렸고 승인됨).
- `nonisolated(nonsending)`은 맹글링으로 구분되지 않으므로 **선언으로만** 구분한다.
- sync `isolated` 함수에는 **현재 Task의 isolation만** 넘길 수 있다.

## 2. 목표, 비목표, 반목표

목표 (단계 순서대로)
1. `isolated` 매개변수와 `#isolation`: 현재 isolation 값을 만들어 넘긴다. 맹글러가 `isolated`를 처리한다.
2. actor의 sync 메서드를 Rust에서 `await`로 부른다(그 actor로 옮겨 가 실행하고 돌아온다).
3. typed throws(`throws(E)`): 구체 에러 타입과 제네릭 에러 타입.
4. `SwiftStream`이 macOS 15 이상에서 `next(isolation:)`을 쓴다(없으면 기존 `next()`).
5. `nonisolated(nonsending)` 선언: 숨은 isolation 인자를 앞에 넣는다.

비목표
- Rust 타입을 Swift가 `await`할 수 있는 actor로 내놓기(Rust executor를 Swift executor로).
- 임의의 actor를 isolation 값으로 만들기(현재 Task의 isolation만). global actor는 MainActor만.
- `.swiftinterface`를 읽어 선언을 대조하는 검사(나중 과제).
- `#isolation` 기본값을 매크로가 자동으로 채우기: 1단계에서는 Rust 시그니처에 `Isolation` 매개변수를 명시한다.

반목표
- 선언 없이 심볼만으로 `nonsending` 여부나 에러 전달 방식을 추측하는 동작.
- 다른 actor의 isolation을 sync `isolated` 함수에 넘길 수 있는 API.
- 런타임 내부 의존을 늘리는 구현: 옮겨 가기는 공개 진입점 `swift_task_switch`와 `Actor.unownedExecutor`로만 한다.

## 3. 실험으로 확인한 ABI (요약)

| 대상 | 저수준 모양 |
|---|---|
| `isolated (any Actor)?` | 정수 인자 2개: actor 객체, `Actor` witness table. nil은 `(0, 0)`. async 함수는 시작할 때 스스로 그 actor로 옮겨 감 |
| actor sync 메서드 | 평범한 swiftcc `(args…, swiftself actor)`. 바깥에서 부르는 쪽이 `swift_task_switch(ctx, resume, executor)`로 옮긴 뒤 호출. default actor executor는 `(actor, 0)` |
| typed throws, 구체 에러 | 에러 **값**이 결과 레지스터로, `swift_context` 레지스터는 던졌음 플래그(null이 아니면 던짐) |
| typed throws, 제네릭 에러 | 호출자가 제네릭 인자 뒤에 에러 슬롯 포인터를 하나 더 넘기고, 피호출자가 거기에 에러를 씀. 플래그는 같음 |
| `next(isolation:)` | `(간접 결과 Element?, ctx, iso.0, iso.1, 에러 슬롯, swiftself 반복자 주소, Self 메타데이터, AsyncIteratorProtocol witness table)` |
| `nonisolated(nonsending)` | 숨은 isolation 인자 2개가 자기 인자 **앞에**. 맹글링은 일반 async 함수와 같음 |

## 4. 설계

### 4.1 `Isolation` (단계 1)

- `apus::swift::Isolation`: "이 호출이 실제로 실행되는 곳의 isolation". 값은 `Isolation::current()`로만 만들고, 데이터가 없는
  표시일 뿐이다(다른 actor를 담을 수 없음).
- **실행 시점에 정한다**(빌더 체크 반영): 인자를 준비할 때가 아니라 funclet이 호출을 실행할 때 두 레지스터를 채운다.
  - 같은 Task에서 실행되고 그 Task가 MainActor Task(`spawn_on_main`, `spawn_local_on_main`, `run_main`의 main task)면 → MainActor
    (`MainActor.shared`와 MainActor의 `Actor` 적합성 witness table)
  - 자식 Task(`join!`, `.child()`, `timeout`, `SwiftStream::child()`)나 일반 Task에서 실행되면 → nil `(0, 0)`.
    Swift의 `async let` 자식도 nonisolated로 실행되므로 같은 의미다.
  - 그래서 자식 Task에서 "MainActor에 있다"고 거짓으로 알리는 일이 없고, `Isolation`을 다른 스레드로 옮겨도 문제가 없다.
- 헤더에 추가: MainActor 메타데이터 접근자 `$sScMMa`(APUS_DIRECT), `MainActor.shared` getter `$sScM6sharedScMvgZ`
  (`swiftself` 메타데이터를 받으므로 C와 기계어가 다름 → APUS_THUNK; `lowering`이 판정), witness table 데이터 심볼 `$sScMScAsWP`.
- 맹글러: 매개변수 앞의 `isolated` 키워드를 읽어 매개변수 타입 뒤에 `Yi`를 붙인다. `= #isolation` 기본값은 읽고 무시한다.
  `(any Actor)?` 타입은 이미 맹글링된다(`ScA_pSg`). swiftc 대조 코퍼스에 사례를 추가한다.
- 선언 예: `#[swift_linked = "Probe.withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int"]`
  `fn with_isolation(x: i64, isolation: Isolation) -> i64;`

### 4.2 actor sync 호출 (단계 2)

- 맹글러는 `(actor)`를 클래스로 맹글링하되, 종류가 actor라는 정보를 잃지 않고 매크로에 알려 준다.
- isolation이 필요한 sync 선언(빌더 체크 반영, 설계자 판단):
  - actor의 sync 메서드와 프로퍼티 getter → 그 actor로 옮겨 감
  - `@MainActor` 표시가 붙은 sync 함수·메서드(선언 문자열 앞에 `@MainActor`) → main executor로 옮겨 감
  - `nonisolated` 표시가 붙은 actor 멤버 → 옮기지 않음(지금처럼 바로 호출 가능)
- 이런 선언은 Rust에서 `async fn`으로 선언하고 매크로가 "옮겨 가는 sync 호출"(`SwiftCall`)을 만든다. Rust에서 sync로 선언하면
  컴파일 오류(바깥에서 isolation 없이 부르는 것이므로).
- 같은 Task 경로와 자식 Task 경로 모두 옮겨 가고 돌아온다(자식은 자기 executor, 즉 generic으로 돌아옴).
- executor 구하기(모두 공개 ABI): 객체의 메타데이터(`swift_getObjectType`) → `Actor` 적합성(`swift_conformsToProtocol`,
  프로토콜 디스크립터 `$sScAMp`) → `Actor.unownedExecutor` getter의 dispatch thunk(`$sScA15unownedExecutorScevgTj`)를
  불러 `UnownedSerialExecutor`(정수 2개)를 얻는다. default actor는 `(actor, 0)`이 나오고, custom executor actor도 같은 경로다.
  적합성이 없으면 `CallError::NotAnActor`.
- funclet: 호출 레코드에 대상 executor가 있으면 `swift_task_switch(ctx, hop_resume, executor)` → `hop_resume`에서
  기존 sync 트램폴린으로 호출 → `swift_task_switch(ctx, after_hop, Task의 executor)` → 결과를 기록하고 원래 흐름으로.
  "Task의 executor"는 `RsTaskContext.executor`(generic 또는 main)다.

### 4.3 typed throws (단계 3) — 소유자 결정: A (ABI가 고정된 경우만)

- 빌더 체크 실험: 구체 에러 타입은 에러·결과 레이아웃의 조합에 따라 레지스터(정수/실수가 섞이기도 함)나 슬롯으로 온다.
  이는 IRGen 규칙이라 선언만으로 알 수 없고, 잘못 추측하면 메모리가 조용히 깨진다.
- 그래서 1차는 **에러 타입이 제네릭 매개변수나 연관 타입인 경우만** 지원한다. 이때는 항상 호출자가 에러 슬롯 포인터를 제네릭 인자
  뒤에 넘기고(실험 확인), 플래그(`RetRegs.error`가 null이 아님)로 던졌는지 안다.
- 맹글러: `throws(E)`는 에러 타입을 맹글링한 뒤 `YK`(`throws`는 `K`). `throws(any Error)`는 `throws`와 같은 의미로 `K`.
- Rust: 슬롯은 기존 `IndirectSlot`처럼 상자로 만들고(에러 타입 메타데이터로 할당), 던지면 값을 `SwiftValue`로 꺼내
  `CallError::Typed(SwiftValue)`로 돌려준다. 에러 타입 메타데이터는 호출자가 넘기는 제네릭 인자에서 얻는다.
- 구체 에러 타입의 `throws(E)` 선언은 매크로가 컴파일 오류로 막는다("concrete typed throws is not supported yet: its error
  may come back in registers or through a slot depending on layout"). 나중 과제.
- sync typed throws(예: actor sync 메서드의 `throws(E)`)도 같은 규칙: 제네릭 에러만, sync 트램폴린에 슬롯 인자를 추가.

### 4.4 `SwiftStream`과 `next(isolation:)` (단계 4)

- `apus_sys::optional`로 `next(isolation:)` dispatch thunk(`…FTjTu`)를 찾는다. 있으면 그것을, 없으면 기존 `next()`를 쓴다.
- 인자: `Isolation::current()`(4.1처럼 실행 시점에 정함, 자식 모드면 nil), 에러 슬롯(반복자의 `Failure` 연관 타입:
  `swift_getAssociatedTypeWitness`로 메타데이터를 구함), 반복자 주소, 메타데이터, witness table. `Failure`가 `Never`면 슬롯은 쓰이지 않는다.
- 동작은 바뀌지 않는다(같은 값, **같은 에러 표현**): 슬롯의 `Failure` 값을 `swift_allocError`로 `any Error` 상자에 넣어
  지금처럼 `CallError::Swift(SwiftError)`로 돌려준다. `Failure: Error` 적합성은 `swift_getAssociatedConformanceWitness`로 구한다
  (둘 다 공개 런타임 함수). macOS 14에서는 기존 경로.

### 4.5 `nonisolated(nonsending)` (단계 5)

- 선언 문자열 앞의 `nonisolated(nonsending)`을 맹글러가 읽고 맹글링에는 반영하지 않는다(실험 결과).
- 매크로는 그 선언의 호출에서 숨은 isolation 인자(`Isolation::current()`)를 맨 앞에 넣는다.
- 위험: 선언을 빠뜨리면 인자가 밀린 채 조용히 잘못 동작한다(6절). README에 경고한다.

## 5. 검증 (적게)

- 맹글러: swiftc 대조 코퍼스에 `isolated` 매개변수, `throws(E)`(구체/제네릭), actor 메서드, `nonisolated(nonsending)` 사례 추가.
- probe Swift + 테스트 하나(`tests/swift_isolation.rs`):
  - `withIsolation`이 nil에서 x, MainActor Task에서 x+1000을 돌려준다.
  - default actor와 custom executor actor의 sync 메서드가 그 actor 위에서 실행된다(Swift 쪽 `assumeIsolated` 또는 executor 비교).
  - typed throws: 구체 에러(값 확인), 제네릭 에러(슬롯 값 확인), 정상 반환.
  - `SwiftStream`(AsyncStream, AsyncThrowingStream)이 macOS 15에서 `next(isolation:)` 경로로 기존과 같은 결과.
  - `nonisolated(nonsending)` 함수가 올바른 인자를 받는다.
- 기존 테스트 전부, `lowering`, `fresh`, `symbols` 통과.

## 6. ABI로 보장하기 어려운 것 (소유자에게 알림, 승인)

| # | 내용 | 대응 |
|---|---|---|
| 1 | `nonisolated(nonsending)`은 맹글링이 같아 심볼로 구분 불가 | 선언으로만 구분, README 경고, `.swiftinterface` 대조는 나중 |
| 2 | 구체 에러 타입은 레이아웃에 따라 레지스터/슬롯이 갈림(빌더 체크) | 소유자 결정 A: 제네릭·연관 타입 에러만 지원, 구체 에러 선언은 컴파일 오류 |
| 3 | custom executor actor | `Actor.unownedExecutor`로 일반 처리(ABI 보장), 적합성은 런타임에서 찾음 |
| 4 | sync `isolated` 함수에 다른 actor를 넘기면 data race | `Isolation`은 `current()`로만 생성 |
| 5 | funclet의 기존 런타임 내부 의존 | 이번에 늘리지 않음(옮겨 가기는 `swift_task_switch`만) |

## 7. 설계자가 정한 것 (빌더 체크 반영)

- `Isolation`은 실행 시점에 정한다(자식 Task에서는 nil). 그래서 `!Send`로 만들 필요가 없다.
- `@MainActor` sync 선언과 `nonisolated` actor 멤버, actor 프로퍼티 getter도 단계 2에 포함한다.
- `next(isolation:)`의 에러는 `any Error`로 상자에 넣어 기존과 같은 `SwiftError`로 돌려준다.
- `nonsending`의 숨은 인자 위치: 간접 결과 포인터 뒤, 자기 인자 앞, `self`는 자기 레지스터(빌더가 실험으로 확인).

## 8. 빌더에게 맡기는 것

- 매크로가 "actor sync 호출"을 고르는 방법(proc macro가 선언에서 kind를 알려 주는 보조 매크로 등).
- 호출 레코드에 대상 executor를 담는 방식, funclet 함수 이름.
- 에러 슬롯 할당 방식(기존 `IndirectSlot` 재사용 권장).
