# 제네릭 인자 자동, `any P` 호출, Rust 클로저를 Swift로

작성일: 2026-11-17 · 상태: 설계 (승인 대기) · 실험: `spikes/m5-isolation/NOTES.md`, `spikes/m6-closures/NOTES.md`

## 1. 의도

소유자 요청: "제네릭, 프로토콜, 에러"(enum 제외, D=Rust로 Swift 프로토콜 구현은 보류) 중 남은 A, B와, `async_main`에서 생긴
"Rust future를 Swift async 클로저로" 형태를 일반화하는 것(덤 케이스, E).

## 2. 범위와 순서

| 단계 | 내용 |
|---|---|
| A | 제네릭 인자(메타데이터, witness table)와 typed throws 에러 슬롯을 `swift!`가 선언에서 자동으로 채움 |
| B | `any P` 값(existential)의 요구사항 호출 |
| E | Rust 클로저(async와 sync)를 Swift 클로저 값으로 Swift API에 넘김 |

비목표: enum, Rust로 Swift 프로토콜 구현(D), Rust에서 새 Swift 에러 타입 만들기(D에 의존; E의 클로저는 Swift에서 받은 `SwiftError`만
다시 던질 수 있음), 연관 타입에 걸린 적합성 요구(`C.Element: Hashable`)의 자동 채우기.

반목표: 선언 없이 추측하는 ABI(특히 E의 클로저 규약), 자동 채우기 실패를 조용히 넘어가는 동작.

## 3. A. 제네릭 인자 자동 채우기

- 지금: 제네릭 함수의 Rust 시그니처에 `ty: SwiftType`, `w: WitnessTable`(그리고 typed throws면 `ErrorSlot`)을 ABI 순서대로 직접 둔다.
- 자동 모드: Rust 시그니처가 Swift 선언의 **형식 매개변수만** 가지면(개수가 같으면) 매크로가 뒤에 붙인다. 개수가 많으면 지금처럼
  명시 모드(기존 코드는 그대로 동작).
- 채우는 순서(ABI, 맹글러가 이미 swiftc와 대조한 정준 순서): 제네릭 매개변수마다 메타데이터 → 정준 요구사항 순서의 witness table →
  (typed throws면) 에러 슬롯.
- 값 구하기:
  - 제네릭 매개변수 `T`의 메타데이터: 선언에서 타입이 정확히 `T`인 형식 매개변수의 Rust 인자(`&SwiftValue`)의 `ty()`.
    그런 매개변수가 없으면(`T`가 결과에만 나오는 등) 자동 모드는 컴파일 오류로 명시 모드를 안내한다.
  - `T: P` witness table: 런타임 적합성 조회(`swift_conformsToProtocol`). 프로토콜 디스크립터 심볼은 맹글러가 만든다(`…Mp`).
    적합하지 않으면 호출하지 않고 `CallError::NotConforming(타입, 프로토콜)`.
  - 연관 타입 적합성 요구, 같은 타입 요구 외의 복잡한 요구는 자동 모드에서 컴파일 오류(명시 모드 안내).
- ABI: 메타데이터와 적합성은 런타임이 주고, 순서는 맹글러 코퍼스로 검증된 규칙이다.

## 4. B. `any P` 호출

- existential 컨테이너 레이아웃(Swift ABI 문서, 안정): 버퍼 3워드 + 동적 타입 메타데이터 + 프로토콜마다 witness table. 값이 버퍼에
  들어가지 않으면(값 witness 플래그 IsNonInline) 버퍼 0번이 상자이고 값은 상자 헤더 뒤(정렬 반영).
- `SwiftValue`가 `any P` 타입이면 `SwiftValue::open() -> Opened { value 주소, 타입, witness tables }`로 연다.
- 요구사항 호출: `swift!`에서 `Module.P(protocol).req(...)` 선언, Rust 수신자 `self: &Opened`. 호출은 요구사항의 dispatch thunk
  (`…Tj`)로 하고 인자 뒤에 Self 메타데이터와 그 프로토콜의 witness table을 붙이며 self는 값 주소(context 레지스터, 실험 M3 P2).
- `Tj`는 resilient 모듈(Apple 프레임워크, library evolution)에만 있다. 없으면 `CallError::Unavailable`(non-resilient 모듈의 표 위치
  세기는 하지 않음: ABI로 보장하기 어려움).

## 5. E. Rust 클로저를 Swift 클로저로

- 값: 2워드(함수, context). 함수는 우리 funclet(async면 `AsyncFunctionPointer`), context는 Swift 힙 객체(우리가 C에 정의한 힙
  메타데이터: HeapLocalVariable 종류, destroy 함수가 Rust 클로저를 drop하고 `swift_deallocObject`).
- 범용 수신: funclet은 모든 인자 레지스터(정수 8, 실수 8, arm64)와 context를 받는 모양이라 어떤 시그니처의 호출도 받는다(호출 쪽
  범용 trampoline의 반대).
- Rust API: `SwiftClosure::new_async(|args: ClosureArgs| async move { … })`, `SwiftClosure::new_sync(...)`. `ClosureArgs`에서
  `arg::<T>(i)`/`indirect(i)`로 꺼내고, 결과는 `ClosureReturn`(정수/실수 레지스터, 또는 간접 결과 주소에 쓰기)으로 돌려준다.
  `swift!`가 선언의 함수 타입을 보고 타입 있는 래퍼를 만든다(예: `(Int) async -> Int` → `Fn(i64) -> impl Future<Output = i64>`).
- 실행: async 클로저는 호출자 Task 안에서 Rust future를 돌린다(`async_main`과 같은 `start` 경로, 호출마다 Rust Task 객체 하나).
  `@MainActor` 클로저는 시작할 때 main executor로 옮겨 간다(호출자가 옮기지 않으므로).
- 에러: 클로저가 `Err(SwiftError)`를 내면 그 상자를 swift_context로 던진다. 새 에러 타입은 비목표(D).
- 맹글러: 함수 타입 매개변수(`(Int) async throws -> String`, `@escaping`, `@Sendable`, `@MainActor`)를 맹글링한다(코퍼스 대조).

## 6. ABI로 보장하기 어려운 것 (진행 전에 알림)

| # | 내용 | 대응 |
|---|---|---|
| 1 | **E의 클로저 규약은 선언된 매개변수 타입의 추상화 수준에 따라 바뀐다**: 구체 타입이면 값이 레지스터, 제네릭(`(T) -> T`)이면 주소. 같은 클로저라도 선언이 틀리면 조용히 깨진다 | 선언의 함수 타입에서만 정한다(맹글러가 구체/제네릭을 앎). 추측하지 않는다 |
| 2 | E의 context 객체는 우리가 C에 만든 힙 메타데이터(HeapLocalVariable)를 쓴다. 참조 카운트와 해제 규약은 ABI지만, 이런 객체를 리플렉션(`Mirror`)하면 캡처 정보가 비어 보인다 | 동작에는 영향 없음. 문서에 적음 |
| 3 | E의 async 클로저는 **다른 Task 안에서** 우리 실행 경로(funclet, 취소 핸들러 등록)를 돌린다. `async_main`에서 확인했지만 동시 다중 호출, 취소, 자식 Task는 새로 검증해야 한다 | 테스트로 동시 호출, 취소, `.child()` 경로를 확인 |
| 4 | B의 `Tj` dispatch thunk는 resilient 모듈에만 있다 | 없으면 `Unavailable`. 표 위치 세기(보장 약함)는 하지 않음 |
| 5 | A의 자동 모드는 형식 매개변수 개수로 명시 모드와 구분한다. 개수가 우연히 맞는 잘못된 선언은 기존과 같은 위험 | 자동 모드에서 타입을 못 구하면 컴파일 오류 |

## 7. 검증 (적게)

- 맹글러 코퍼스: 함수 타입 매개변수 사례들.
- 테스트 파일 하나씩: `tests/swift_generic_auto.rs`(A), `tests/swift_existential.rs`(B, resilient 모듈이 필요하므로 probe를
  `-enable-library-evolution`로 빌드하는 작은 Swift 모듈을 하나 추가), `tests/swift_closures.rs`(E: 구체/제네릭/sync/@MainActor/
  throw/동시 호출/취소).
