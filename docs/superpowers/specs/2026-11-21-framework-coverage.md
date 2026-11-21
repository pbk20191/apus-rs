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

| 모듈 | 처음 | extension, stdlib 이름 | 중첩 타입 | ObjC 클래스, 속성 | 기본값, typealias, 팩 | 옮겨진 타입, builder |
|---|---|---|---|---|---|---|
| StoreKit | 265/804 | 343/804 | 524/804 | 590/804 | 602/804 | 602/804 |
| SwiftData | 268/505 | 347/505 | 383/505 | 390/505 | 425/505 | 426/505 |
| MusicKit | 657/974 | 828/974 | 857/974 | 861/974 | 862/974 | 864/974 |
| WeatherKit | 187/382 | 267/382 | 275/382 | 335/382 | 336/382 | 336/382 |
| AppIntents | 480/3771 | 654/3771 | 665/3771 | 678/3771 | 749/3771 | 751/3771 |
| SwiftUI | | | | | 1358/2956 | 1584/2956 |
| Foundation | | | | | 3001/9848 | 3020/9848 |

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

- 옮겨진 타입(`@_originallyDefinedIn`):
  - 예: `CoreFoundation.CGFloat`는 CoreGraphics에 정의되어 있었고, SwiftUICore에는 이런 표시가 6597개 있다.
  - 심볼은 원래 모듈 이름으로 mangle된다. graph에는 이 속성이 없고 USR도 지금 모듈을 쓴다.
  - 그래서 SDK의 `.swiftinterface`에서 이 속성이 붙은 맨 위 수준 타입을 모아 두고, mangler가 원래 모듈 이름을 쓴다.
    `KindResolver::original_module`로 한다.
  - SDK 검사는 선언에 적힌 모듈의 `.tbd`를 그대로 본다. 옮겨진 타입의 심볼은 지금 모듈의 바이너리에 있기 때문이다.
  - 이름으로 쓴 선언의 심볼이 USR과 다르더라도 SDK가 그 심볼을 export하면 받아들인다.
- 속성 처리:
  - 파라미터의 result builder(`@ContentBuilder actions:`)는 컴파일 시점 기능이므로 지운다.
  - `@export(implementation)`와 `dynamic`도 지운다. 심볼은 `.tbd`로 검사한다.
- `Void`가 typealias 분기에 먼저 걸리던 것을 고쳤다.
- 가져온 C 타입과 Objective-C 클래스의 Swift extension:
  - C 타입은 C 이름으로 `__C`에 있는 것처럼 mangle된다(Mangling.rst "Imported C and C++ structs"는 struct의 `V`를
    문서화한다). 나머지는 swiftc로 확인했다(2026-11-21): `NS_ENUM`과 `NS_OPTIONS`는 `So…V`, `NS_TYPED_ENUM`과
    익명 struct는 typedef 이름으로 `So…a`, 보통 typedef(`TimeInterval`)는 원래 타입(`Sd`)이다.
  - graph가 USR(`c:@S@`, `c:@E@`, `c:@SA@`, `c:@T@`)과 종류(struct, class, typealias)를 알려 주므로 그대로 다시 쓴다.
    모듈이 붙은 USR(`c:@M@_DarwinFoundation@E@POSIXErrorCode`)은 `Darwin.POSIXErrorCode`로 mangle되므로 거부한다.
  - Objective-C 클래스나 C 타입 안에 모듈이 선언한 것은 extension 표시를 단다:
    `So9NSRunLoopC10FoundationE17SchedulerTimeTypeV`, `So9NSDecimala10FoundationE11FormatStyleV`.
  - 결과는 USR과 `.tbd`로 검사하므로 이름이 틀리면 컴파일 오류다.
  - 이런 C 값은 C 배치대로 넘어가므로 `SwiftValue`로 선언하면 컴파일 오류로 거부한다(전에는 검사가 없어 UB였다).
  - coverage는 Swift 심볼이 없는 Objective-C/C 선언(`c:` USR)을 따로 센다.
  - 결과: Foundation 3020 → 3512/5127(Objective-C/C 선언 4721개 제외), SwiftUI 1584 → 1688.

## 남은 큰 항목

- 생성자에 붙은 `where`가 타입 파라미터를 고정하는 경우(`init(…) where Message == EmptyView`). 넘기는 generic 인자가
  달라지므로 지금처럼 거부한다. SwiftUI에서 많다.
- Foundation: "export 안 됨" 2878개. 일부는 `@inlinable`이다. 일부는 `_System_Foundation` 같은 다시 내보낸 모듈의
  extension인데, graph는 Foundation의 멤버로 보여 준다. 예: `URL.init(_: FilePath)`.

- 모듈이 붙은 C 타입(`POSIXErrorCode`, `MachErrorCode`): 약 160개.
- 프로토콜의 associated type(`…Qa`, `Self.Filter`)과 stdlib typealias.
- export되지 않은 것(`@inlinable` 등): shim이 필요하다.
- AppIntents: result builder와 string interpolation 같은 컴파일 시점 기능, 그리고 "parse: expected …".
