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

| 모듈 | 처음 | extension, stdlib 이름 | 중첩 타입 |
|---|---|---|---|
| StoreKit | 265/804 | 343/804 | 524/804 |
| SwiftData | 268/505 | 347/505 | 383/505 |
| MusicKit | 657/974 | 828/974 | 857/974 |
| WeatherKit | 187/382 | 267/382 | 275/382 |
| AppIntents | 480/3771 | 654/3771 | 665/3771 |

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

## 남은 큰 항목

- 가져온 C/Objective-C 타입(`c:` USR, `So…` 심볼): rewrite가 거부한다.
- typealias(`…a` USR)를 풀지 못한다.
- `Foundation.Predicate`처럼 파라미터 팩을 받는 타입(`_QP`): mangler가 팩을 모른다.
- export되지 않은 것(`@inlinable` 등): shim이 필요하다.
- AppIntents: result builder와 string interpolation 같은 컴파일 시점 기능, 그리고 "parse: expected …".
