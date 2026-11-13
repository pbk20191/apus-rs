# Swift async 함수 직접 호출 설계 (제안)

작성일: 2026-11-13 · 상태: 1단계와 제네릭 확장 구현 완료, 2~3단계 남음 (아래 7절) · 실험: `docs/superpowers/specs/spikes/m1/` (`./build.sh && ./m1 --m1`)

## 1. 의도

지금은 Swift async 코드를 부르려면 사용자가 Swift 쪽에 레시피(`AsyncSwiftClosure`)와 `@_cdecl` 팩토리를 써야 한다.
이 기능은 그 Swift 코드를 없앤다. **Rust에서 Swift async 함수를 심볼로 찾아, 인자를 넘겨, 호출한 Task 안에서 직접 실행한다.**
StoreKit 2처럼 Swift에만 있는 API를 Rust에서 바로 쓰기 위한 것이고, cidre의 `#[swift::call("…")]`에서 얻은 아이디어다.

cidre와 다른 점: 호출이 **별도 Task가 아니라 호출한 Rust 태스크와 같은 Swift Task**에서 실행된다
(취소, 우선순위, task-local이 그대로 이어짐).

## 2. M1 실험 결과: PASS

arm64 30/30, x86_64(Rosetta) 20/20. 기존 M0 시나리오도 그대로 통과.

| # | 호출 | 결과 |
|---|---|---|
| F1 | `addAsync(3, 4) async -> Int` | 7 |
| F2 | 호출된 함수의 현재 Task | 호출한 Rust 태스크와 같음 |
| F3 | `greet("Kim", times: 2, sleepMs: 10) async throws -> String` | `"Kim Kim "`, 에러 없음 |
| F4 | `half(3.0, plus: 10) async -> Double` | 11.5 (실수 인자와 결과) |
| F5 | `failing(code: 5) async throws` | `DemoError.code(5)` |
| F6 | `names(count: 3) async -> [String]` | `["name0", "name1", "name2"]` |
| F7 | `maybe(_:) async -> Int?` | `Some(40)`, `None` |
| F8 | `Counter.bump(by:)` (클래스 메서드, self) | 5, 10 |
| F9 | MainActor 태스크에서 호출 | 호출 뒤 MainActor로 복귀 |
| F10 | 호출 중 Task 취소 | `CancellationError` |
| F11 | **OS에 들어 있는 `Task.sleep(nanoseconds:)`를 `dlsym`으로 찾아 호출** | 52ms 뒤 정상 반환 |

테스트용 Swift 함수는 평범한 `public func`이다. `@_cdecl`도 글루도 없다. F11은 우리가 만든 Swift 코드가 전혀 관여하지 않는다.

### 핵심 발견: 어셈블리가 필요 없다

- **범용 trampoline 하나로 모든 시그니처를 호출한다.** C 함수 타입 하나(정수 레지스터 전부 + 실수 레지스터 8개 +
  async 컨텍스트 + self)로 callee를 tail call하면, callee는 자기 시그니처가 쓰는 레지스터만 읽는다.
  Rust는 인자를 "레지스터 표"에 채우기만 한다. 시그니처별 코드 생성이 없다.
- **결과도 범용 continuation 하나로 받는다.** async 함수의 직접 반환값은 반환 continuation의 **인자**로 온다
  (정수 4개, 실수 4개까지). 에러는 `swift_context` 레지스터로 온다.
- clang `swiftasynccall`로 작성했으므로 **arm64와 x86_64에서 같은 C 코드**가 동작한다 (정수 인자 레지스터 수만 8/6으로 다름).
- 따라서 기존 스펙의 반목표("손으로 쓴 어셈블리 금지")를 그대로 유지할 수 있다.
  사용자가 "안정적이면 어셈블리도 좋다"고 했으므로, 나중에 C로 표현할 수 없는 경우가 나오면 그때 해당 부분에만 쓴다.

### 확인한 호출 규약 (구현이 따를 것)

- async 함수 포인터 심볼은 mangled 이름 + `Tu`다. `{ int32 코드 상대 오프셋, uint32 컨텍스트 크기 }`.
- 인자: 정수/포인터는 정수 레지스터 순서대로, `Double`은 실수 레지스터 순서대로. `String`은 정수 레지스터 2개(+0, guaranteed).
  메서드의 self는 `swift_context`.
- 반환: `Int`/`Bool`/포인터는 정수 0번, `Double`은 실수 0번, `String`은 정수 0~1번(+1 소유),
  `Array`는 저장소 포인터(+1), `Int?`는 정수 0번(값) + 1번의 최하위 비트(1이면 nil).
- 호출이 끝나면 MainActor 태스크는 `swift_task_switch`로 복귀한다 (M0 규칙과 동일).

## 3. 제안하는 구성

**1단계 (이번 범위 제안): 심볼로 호출하는 타입 있는 API**

```rust
// 심볼은 `nm` / `swift demangle`로 얻는다. 런타임에 dlsym으로 찾으므로, 없는 OS에서는 None이 된다.
static GREET: SwiftAsyncFn = SwiftAsyncFn::new("$s5Probe5greet_5times7sleepMsS2S_S2itYaKF");

let text: String = GREET.call(("Kim", 2i64, 10i64)).throws().await?;   // 같은 Task에서 실행
let n: i64 = BUMP.method(&counter).call((5i64,)).await;
```

- 인자 타입(`SwiftArg`): `i64`/`i32`/`u64`/`bool`, `f64`, `&str`/`String`, Swift 객체 참조.
- 결과 타입(`SwiftRet`): `()`, 정수, `bool`, `f64`, `String`, `Vec<T>`, `Option<정수>`, Swift 객체 참조.
  `SwiftAny`의 `from_value`(이번에 추가한 Array/Dictionary 읽기)를 그대로 재사용한다.
- `throws()`를 붙이면 `Result<R, SwiftError>`. 붙이지 않은 함수의 에러 레지스터는 읽지 않는다.
- 레지스터에 다 들어가지 않는 시그니처(정수 인자가 arm64 8개, x86_64 6개 초과)는 호출 전에 에러로 거절한다.
- 동시 실행 규칙은 `SwiftAsync`와 같다: 단독 await는 같은 Task, `.child()`/`timeout()`은 자식 Task.
- 기존 `SwiftAsync`(클로저 레시피)는 그대로 둔다. 인자를 Swift 쪽에서 캡처하는 편이 쉬운 경우에 여전히 유용하다.

**2단계 (다음): 선언 문자열에서 심볼 계산**

```rust
#[swift_fn("Probe.greet(_: String, times: Int, sleepMs: Int) async throws -> String")]
async fn greet(name: &str, times: i64, sleep_ms: i64) -> Result<String, SwiftError>;
```

proc macro가 Swift mangling 규칙으로 심볼을 계산한다 (별도 크레이트 `async-swift-rs-macros`).
mangling 구현이 큰 작업이라 1단계와 분리한다.

**3단계 (다음): `AsyncSequence` → Rust `Stream`** (StoreKit 2 `Transaction.updates` 등). 1단계 위에 올린다.

## 4. 1단계에서 하지 않는 것

- 제네릭 함수(metadata와 witness table 인자), 큰 값이나 resilient struct의 간접 반환(예: `[Product]`의 `Product` 값 읽기),
  클로저 인자, 스택으로 넘어가는 인자, typed throws, `isolated` 파라미터. 필요해지면 실험부터 한다.
- 심볼 자동 계산(2단계).

## 5. 위험

- 반환 continuation의 레지스터 규약은 컴파일러가 생성한 코드에 맞춘 것이다. Swift ABI 안정성 덕분에 고정이지만 공개 문서화된 계약은 아니다.
  기존 자체 점검에 "알려진 함수(`Task.sleep`) 한 번 호출" 검사를 추가해 대비한다.
- 잘못된 시그니처(인자 타입을 틀리게 선언)는 컴파일러가 잡아 주지 못한다. 2단계의 선언 기반 매크로가 이 위험을 줄인다.

## 6. 결정이 필요한 것

1. 1단계(심볼 문자열로 호출)부터 구현하고, 2단계(선언에서 심볼 계산)는 그다음에 하는 순서로 갈지.
2. 1단계의 타입 범위가 위 목록으로 충분한지. 특히 StoreKit 2를 바로 쓰려면 4절의 "간접 반환"과 "제네릭"이 필요해서,
   그걸 1단계에 포함할지 여부.

## 7. 진행 상황 (2026-11-13)

사용자가 범위를 넓히기로 했다: 매크로, 제네릭, metadata, witness table까지 포함한다.

**구현 완료**
- 1단계: `SwiftAsyncFn`, `SwiftCall`, 인자/결과 변환 (`tests/swift_fn.rs`, 17개).
  클로저 호출(`SwiftAsync`)도 같은 범용 호출 경로를 쓰도록 통합했다.
- 제네릭 확장 (실험: `spikes/m2/`, arm64 20/20, x86_64 10/10; 테스트: `tests/swift_generic.rs`, 8개):
  - `SwiftType`(metadata), `SwiftProtocol`/`WitnessTable`(`swift_conformsToProtocol`), `SwiftValue`(임의 타입의 소유 값).
  - 제네릭 함수: 값은 주소로, 그 뒤에 타입 인자 metadata, 그 뒤에 요구 conformance마다 witness table.
  - 주소로 넘기고 받는 값(제네릭, 큰 struct, 프레임워크의 resilient struct): `call_returning`, `&SwiftValue`.
  - 동기 함수 호출: `SwiftFn` (getter, 메서드, 자유 함수). 간접 결과 주소는 별도 레지스터(x8 / rax)로 간다.
  - **실제 StoreKit 2 `Product.products(for:)` 호출 확인**: 앱 번들이 없는 테스트 프로세스에서는
    `StoreKitError.systemError(invalidApp)`을 던지며 정상 복귀한다.

**M2에서 확정한 규약**
- async 함수의 간접 결과 주소는 첫 정수 인자다. 동기 함수는 `swift_indirect_result` 레지스터를 쓴다.
- 동기 함수의 직접 결과는 반환 레지스터(정수 4개 또는 실수 4개)로 온다. 정수와 실수가 섞인 결과는 아직 지원하지 않는다.
- `throws` 동기 함수의 에러는 `swift_error_result` 레지스터로 온다. 항상 넘겨도 던지지 않는 함수는 건드리지 않는다.
- 임의 길이 문자열은 `String._fromUTF8Repairing`으로 만든다 (NUL 바이트 포함 가능). 배열은 `_allocateUninitializedArray`로 만든다.
- 타입은 `swift_getTypeByMangledNameInContext`로 mangled 이름에서 얻는다 (없으면 null).

**남은 것**
- 2단계: 선언에서 심볼을 계산하는 매크로. 사용자가 원하는 모습은 `swift-rs`의 `swift!(fn …)`처럼 한 줄 선언이다.
- 3단계: `AsyncSequence` → `Stream`.
- `SwiftValue`의 필드/프로퍼티 접근을 편하게 하는 계층 (지금은 getter 심볼을 `SwiftFn`으로 직접 호출).
