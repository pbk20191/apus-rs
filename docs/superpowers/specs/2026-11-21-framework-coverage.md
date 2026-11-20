# 프레임워크 커버리지 측정 (2026-11-21)

## 방법

- `macros/src/short.rs`의 `coverage` 테스트(`#[ignore]`)를 쓴다.
- SDK symbol graph의 함수, init, property getter, subscript getter마다 다음을 차례로 한다.
  1. 이름으로 쓴 선언처럼 graph에서 다시 쓴다.
  2. parse한다.
  3. mangle해서 graph의 USR과 비교한다.
  4. `swift!`처럼 검사한다(제네릭이면 묶었다고 가정).
  5. SDK `.tbd`에 export되어 있는지 본다.
- 실패 사유를 묶어서 센다.
- 프로토콜 extension의 기본 구현을 타입 멤버처럼 보여 주는 항목(`::SYNTHESIZED::`)은 세지 않는다. 프로토콜로 부르는 것이기 때문이다.

## 결과 (호출 가능 / 전체)

| 모듈 | 처음 | extension, stdlib 이름 | 중첩 타입 | ObjC 클래스, 속성 | 기본값, typealias, 팩 |
|---|---|---|---|---|---|
| StoreKit | 265/804 | 343/804 | 524/804 | 590/804 | 602/804 |
| SwiftData | 268/505 | 347/505 | 383/505 | 390/505 | 425/505 |
| MusicKit | 657/974 | 828/974 | 857/974 | 861/974 | 862/974 |
| WeatherKit | 187/382 | 267/382 | 275/382 | 335/382 | 336/382 |
| AppIntents | 480/3771 | 654/3771 | 665/3771 | 678/3771 | 749/3771 |

## 고친 것 (rewrite, ABI와 무관)

- graph의 `swiftExtension`을 읽는다.
  - 프로토콜 extension, 다른 모듈의 extension, 제약 있는 extension의 멤버를 `(extension in M where …)`로 쓴다.
  - 멤버가 타입의 파라미터에 더하는 요구사항은 멤버의 `where`로 쓴다.
  - 예: `VerificationResult<Transaction>.jwsRepresentation`, `ModelActor.modelContext`.
- stdlib 이름 중 축약 표기가 없는 것(`Decoder`, `Hasher`, `KeyPath`)은 `Swift.`을 붙여 쓴다.
  - 전에는 선언한 모듈의 타입으로 해석되었다. 그래서 `init(from:)`, `encode(to:)`, `hash(into:)`가 USR과 달랐다.
  - 이 차이는 비교 검사로 컴파일 단계에서 거부되었으므로 UB는 아니었다.

- 중첩 타입(`Product.SubscriptionInfo.Item`): graph는 이름 조각마다 fragment를 따로 준다. rewrite가 조각마다 모듈을
  붙여 쓰고 있었다. 이제 안쪽 이름은 kind만 붙여 쓴다.

- Objective-C 클래스(`c:objc(cs)X`): Objective-C 이름으로 `__C.X(class)`라고 쓴다. Swift 이름(`UnitLength`)이 아니다.
  mangling이 Objective-C 이름을 쓰기 때문이다.
- `Void` typealias는 `()`로 쓴다.
- 속성 처리:
  - `@backDeployed`는 무시한다. 심볼은 SDK `.tbd`로 따로 검사한다.
  - 조각으로 나뉜 `@` `MainActor`를 하나로 읽는다.
  - `nonisolated` 속성을 받아들인다.

- 기본값은 지운다(`= #isolation`만 남긴다). 심볼에도 호출에도 영향이 없기 때문이다.
- 모듈의 typealias는 그것이 가리키는 타입으로 쓴다. Swift가 그렇게 mangle한다.
  - 제네릭 typealias와 stdlib typealias(`Array<T>.Index`)는 거부한다.
  - stdlib는 graph가 없다.
- 파라미터 팩을 받는 타입(`Predicate<each Input>`): 그 타입의 graph 선언에 `each`가 있으면 인자를 `Pack{…}`으로 쓴다.
  parser와 mangler는 이미 `Pack{}`을 지원한다.
  - 실험(2026-11-21) 결과는 다음과 같다.
    - `Predicate<Int>`는 `Si_QP`로 mangle된다.
    - `Predicate<Int, String>`은 `Si_SSQP`로 mangle된다.
    - `Predicate<E>`는 `x_QP`로 mangle된다.
  - 파라미터가 팩 하나뿐인 타입만 쓴다.

## 남은 큰 항목

- 가져온 C 타입(enum `c:@E@`, struct `c:@SA@`, typedef `c:@T@`)과 Objective-C에서 온 멤버(`So…` 심볼): rewrite가 거부한다.
- 프로토콜의 associated type(`…Qa`, `Self.Filter`)과 stdlib typealias.
- export되지 않은 것(`@inlinable` 등): shim이 필요하다.
- AppIntents: result builder와 string interpolation 같은 컴파일 시점 기능, 그리고 "parse: expected …".
