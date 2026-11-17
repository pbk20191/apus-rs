# Swift ↔ Rust 상호운용: 조사한 리포지토리

2026-11-13 ~ 11-14 desktop-mcp 세션에서 조사한 결과를 모은 문서다. 별, 라이선스, 마지막 push 날짜는 2026-11-17에 GitHub API로 확인했다.

## 한눈에 보기

| 리포지토리 | 라이선스 | ★ | 방향 | 방식 | 우리와의 관계 |
|---|---|---|---|---|---|
| [chinedufn/swift-bridge](https://github.com/chinedufn/swift-bridge) | Apache-2.0 (MIT 겸용) | 1153 | Rust 호스트 → Swift | 양쪽 글루 코드 생성, Swift는 static lib으로 링크 | 비교 대상. Swift 소스와 C 경계가 필요하다 |
| [Brendonovich/swift-rs](https://github.com/Brendonovich/swift-rs) | Apache-2.0 (MIT 겸용) | 344 | Rust → Swift | `build.rs`로 SwiftPM 빌드·링크, `@_cdecl` 함수 호출 | 비교 대상. 같은 이유로 Swift 소스가 필요하다 |
| [mozilla/uniffi-rs](https://github.com/mozilla/uniffi-rs) | MPL-2.0 | 5143 | Swift 호스트 → Rust | 바인딩 생성기. 반대 방향은 foreign trait(콜백) | 방향이 반대. 동시성은 다루지 않는다 |
| [yury/cidre](https://github.com/yury/cidre) | MIT | 226 | Rust → Apple 프레임워크, Swift 전용 API 포함 | 독자 타입 체계(`arc::R`, `ns::`, `cf::`). Swift async는 arm64 naked 어셈블리 | 참고 자료. 의존성으로는 쓰지 않는다 |
| [eugenehp/rswift](https://github.com/eugenehp/rswift) | GPL-3.0 | 9 | Rust → Swift ABI | `swift-runtime-sys`: Concurrency 함수 약 80개 선언, arm64 asm thunk, 실행기 hook | 설계 수준에서 한 번만 검토. **코드는 가져오지 않는다** |
| [dgrijalva/async-dispatch](https://github.com/dgrijalva/async-dispatch) | Apache-2.0 | 2 | Rust future → GCD | `async-task` runnable을 dispatch 큐에서 실행 | `dispatch` 백엔드의 참고 구현. 가져온 코드에는 출처를 표시한다 |
| [madsmtm/objc2](https://github.com/madsmtm/objc2) | MIT / Apache-2.0 / Zlib | 1089 | Rust → ObjC, Apple 프레임워크 | 프레임워크 바인딩 생성(`dispatch2`, `objc2-core-foundation` 등) | **실제 의존성**. 바인딩은 이 계열을 쓴다 |
| [zachshallbetter/rust-swift](https://github.com/zachshallbetter/rust-swift) | Apache-2.0 | 0 | Rust 소스 → Swift 소스 | 트랜스파일러(Rust의 제한된 부분집합) | 관계없음. 상호운용 도구가 아니다 |

참고한 툴체인 소스:
- [swiftlang/swift](https://github.com/swiftlang/swift): 런타임 ABI, mangling 규칙, 표준 라이브러리 저장소 구조 (6.4.2 기준)
- [swiftlang/llvm-project](https://github.com/swiftlang/llvm-project): `swiftasynccall`, 크로스 언어 LTO 실험(M4)

## 리포지토리별 정리

### swift-bridge
- Rust 바이너리가 호스트이고 Swift 코드를 부르고 싶을 때 가장 가까운 기성 도구다.
- `#[swift_bridge::bridge]` 안에 `extern "Swift"`로 Swift 클래스와 메서드를 선언하면, `String`, `Vec`, `Option`, `Result`, `async`를 자동으로 변환한다.
- Swift 쪽은 평범한 SwiftPM 패키지로 쓰고, `build.rs`에서 static lib으로 빌드해 링크한다.
- 한계는 모든 호출이 생성된 C 경계를 지난다는 것이다. Swift 소스를 따로 써야 하고, Rust future가 Swift Task가 되지 않는다.

### swift-rs
- `build.rs`에서 Swift 패키지를 빌드하고 링크한다. 문자열, 배열, 객체를 넘기는 헬퍼가 있다.
- Swift 쪽에서 `@_cdecl`로 C 진입점을 만들어 Rust에서 부르는 방식이다. 구조는 swift-bridge보다 단순하고 타입 공유는 적다.

### UniFFI
- 기본 구조는 Rust가 라이브러리이고 Swift(또는 Kotlin, Python)가 호스트다.
- Rust에서 Swift를 부를 때는 `#[uniffi::export(with_foreign)]` trait를 Swift 클래스가 구현하는 콜백 방식을 쓴다.
- async, Result, record/enum 매핑이 잘 돼 있고 Firefox iOS 등에서 쓰여 검증이 충분하다.
- 실행 바이너리가 Swift 앱일 때 자연스럽다. 우리 목표인 Rust-main 앱과는 방향이 반대다.

### cidre
- 장점
  - 작성자가 실제 녹화·ML 앱에 쓰면서 다듬은 크레이트다. 성능을 우선한다(정적 문자열, 핫패스 무할당).
  - Swift 전용 API를 ABI로 직접 바인딩한다(DockKit, Speech, AsyncSequence). arm64e 포인터 인증까지 처리한다.
  - `#[swift::call("DockKit.DockAccessoryManager(class).setSystemTrackingEnabled(_: Bool) async throws thunk")]`처럼 Swift 선언 문자열 한 줄로 바인딩을 만든다. 우리 `swift!` / `#[swift = "…"]` 매크로의 출발점이 된 아이디어다.
- 우리와 다른 점
  - Swift async 호출마다 **별도의 Swift Task**를 만들고 우선순위를 Default(`0x15`)로 고정한다. 그래서 호출한 쪽의 우선순위, 취소, task-local이 이어지지 않는다. 우리는 호출한 Rust 태스크와 **같은 Task** 안에서 호출한다.
  - trampoline을 arm64 naked 어셈블리로 직접 썼다. 우리는 clang `swiftasynccall`로 만든 범용 C trampoline 하나를 쓰고, 이 방식은 arm64와 x86_64에서 모두 동작한다.
  - 독자 타입 체계라 objc2 계열과 섞으면 바인딩이 두 벌 생긴다. 이 때문에 `dispatch`·`cf` 바인딩도 cidre 대신 `dispatch2` + `objc2-core-foundation`을 골랐다.
  - 0.x(0.30)라서 변경이 잦다. `main` 브랜치는 Rust 1.100을 요구한다.
- 가져온 아이디어
  - 선언 기반 호출
  - AsyncSequence → `Stream` (`SwiftStream`으로 구현)
  - `Array`/`Dictionary` 변환 (구현)
  - API 가용성 표시 (아직 구현하지 않음)

### rswift (`crates/swift-runtime-sys`)
- 구성
  - `ConcurrencyRuntime.rs`: `libswift_Concurrency` 함수 약 80개를 `extern "C"`로 선언한다.
  - `ConcurrencyThunks.rs`: `swiftcall`/`swiftasync` 진입점을 부르는 arm64 인라인 어셈블리 thunk다. x86_64에서는 그냥 `extern "C"`로 캐스팅한다.
  - `ConcurrencyHooks.rs`: `swift_task_enqueueGlobal_hook` 등으로 전역 실행기를 바꿔 끼운다.
- 한계
  - Rust future를 Swift Task로 만들지 않는다. `Future`, `Waker`, `poll`이 없다.
  - `swift_continuation_await` 같은 `swiftasync` 함수를 반환하는 일반 함수처럼 호출한다. 실제로는 이 함수들이 재개 funclet으로 tail call한다.
  - Swift 호출 규약 함수의 반환값을 Rust `bool`로 읽는다. 정의된 것은 bit 0뿐이라 UB다.
  - 6.4.2 런타임의 `swift_task_switch`는 인자 3개짜리 `swiftasync` 함수인데, 인자 2개짜리 C 함수로 선언한다.
- 결론
  - 가져올 것이 없다.
  - 동기 Concurrency 함수는 다중 워드 struct 반환이 아니면 Swift와 C 규약이 같다는 점은 우리 측정과 일치한다. 그래서 동기 함수는 Rust에서 직접 부르고 인라인되게 한다.
- 라이선스가 GPL-3.0이라 접근 방식만 보았다. 코드는 보지도 가져오지도 않는다는 원칙은 그대로다.

### async-dispatch
- `async-task` runnable을 GCD 큐에 올려 future를 실행하는 작은 크레이트다. 우리 `dispatch` 백엔드가 같은 구조를 따른다.
- 우리는 바인딩을 `dispatch2`로 바꿨다. 깨우기에서 할당이 생기지 않도록 클로저 API 대신 `exec_async_f`를 쓴다.

### objc2 계열
- `dispatch2`(`DispatchRetained`, 큐, 타이머)와 `objc2-core-foundation`(`CFRetained`, `CFRunLoop`)을 쓴다.
- 앞으로 만들 앱도 AppKit, Accessibility, ScreenCaptureKit 등을 `objc2-*`로 쓸 예정이라, 런타임도 같은 타입을 공유하도록 이 계열로 맞췄다.
- 직접 선언은 바인딩이 없는 것만 둔다. 예를 들면 `qos_class_self`, 그리고 Swift 런타임 ABI 자체다.

## 이 프로젝트만 하는 것

| | swift-bridge / swift-rs | UniFFI | cidre | rswift | **이 프로젝트** |
|---|---|---|---|---|---|
| Swift 소스 없이 Swift API 호출 | ✗ | ✗ | ✓ | 일부 | ✓ |
| Rust future = 진짜 Swift Task | ✗ | ✗ | ✗ | ✗ | ✓ |
| Swift async 호출이 같은 Task에서 실행(우선순위·취소·task-local 상속) | ✗ | ✗ | ✗ | ✗ | ✓ |
| arm64 + x86_64 | ✓ | ✓ | arm64 중심 | arm64 asm | ✓ (clang funclet) |
| 선언에서 mangled name 계산 | ✗ | ✗ | ✓ | ✗ | ✓ (swiftc와 110개 대조) |
| 제네릭(metadata, witness table) | 제한적 | 제한적 | 일부 | ? | ✓ |
| 크로스 언어 LTO 인라인 | ✗ | ✗ | ✗ | ✗ | 실험(M4) 통과, 2단계 링크 필요 |
| tokio를 스레드 없는 guest로 | – | – | – | – | 실험 통과(desktop-mcp Rust 포트가 같은 방식 사용) |
