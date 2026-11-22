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
    제네릭 builder(`@ResolverSpecificationBuilder<Value.UnwrappedType> resolvers:`)는 `<…>`까지 지운다(2026-11-23,
    AppIntents 2558 → 2718, SwiftUI 1919 → 1947).
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

- 모듈이 붙은 C 타입(`c:@M@<모듈>@E@<이름>`, 2026-11-21):
  - Swift 모듈에 선언된 `@objc enum`이다: `POSIXErrorCode`는 `_DarwinFoundation1`, `MachErrorCode`는 `Darwin`.
  - 이름은 모듈의 ABI 이름으로 mangle된다(`_DarwinFoundation1`의 `-module-abi-name Darwin`, swiftc로 확인).
    `.swiftinterface`의 flags에서 읽는다.
  - 값은 raw 정수로 레지스터에 넘어가고 돌아온다(swiftc로 확인: 다른 모듈에서 부를 때, library evolution에서도
    `i32`). 문서화된 규칙은 아니어서 owner가 승인했다(2026-11-21). C 값처럼 `CValueArg`/`CValue<i32>`로 다룬다.
  - `@objc` 여부는 선언한 모듈의 graph(USR이 `c:`)로 판정한다. 그 모듈에 없는 이름이면 다시 내보낸 모듈
    (`@_exported import`)을 본다(`Darwin.POSIXErrorCode`).
  - 버그 수정: 전에는 `Darwin.MachErrorCode(enum)`을 전체 선언으로 쓰면 `SwiftValue`(주소)로 넘겨 UB였다. optional C 값
    결과(`CGRect?`)를 `-> CValue<T>`로 받으면 tag를 빼고 읽거나(레지스터) 결과 주소 없이 불러 UB였다. 이제 둘 다 컴파일
    오류다. Foundation 3512 → 3670/4902. `apus::swift::CValue<T>`처럼 경로를 붙인 결과 타입은 거부되던 것도 고쳤다.

- 타입 파라미터를 고정하는 `where`(2026-11-22):
  - swiftc로 확인했다(`-enable-library-evolution -emit-ir`). 고정된 파라미터는 generic 인자에서 빠진다. `where`가
    extension에 붙든 멤버에 붙든 같다.
    - `Two<A, B>.init(b:) where A == Int`는 `B`만 받는다. `init(a:) where B == [A]`는 `A`만 받는다.
    - `One<V>.init() where V == Bool`은 아무것도 받지 않는다. 인스턴스 멤버 `get() where V == Bool`은 `Self` metadata도
      받지 않는다.
    - 일부만 고정된 인스턴스 멤버(`both() where A == Int, B: P`)는 전처럼 `Self` metadata를 받고, 그 뒤에 `B.P`를 받는다.
  - 문서화된 규칙은 아니어서 owner가 승인했다(2026-11-22).
  - 생성자 심볼은 `…tc` + 요구사항 + `l`/`rl` + `u` + `fC`다(`…ycSbRszlufC`). corpus로 swiftc와 비교한다.
  - 버그 수정:
    - 멤버의 `where`로 모두 고정된 인스턴스 멤버에 `Self` metadata를 하나 더 넘겼다.
    - 묶지 않은 static 멤버의 오류 메시지는 고정된 파라미터까지 넘기라고 안내했다. 안내대로 넘기면 UB였다.
  - rewrite 수정: graph에서 다시 쓸 때 타입 자신의 제약도 쓴다(`Form(struct)<Content: SwiftUI.View>`).
    - 타입에 제약이 있으면 멤버의 signature 끝이 `l` 대신 `rl`이 된다.
    - 제약을 빠뜨리면 class-bound 파라미터를 몰랐다. `ResultsObserver<Element: PersistentModel>.element(at:) -> Element?`가
      주소로 받는 결과로 잘못 판단되었다(UB). 이제 거부한다.
  - 결과:
    - SwiftUI 1688 → 1752/2956. mangling 차이는 109 → 36개다.
    - Foundation 3670 → 3699/4902.
    - SwiftData 431/505. 타입 제약을 쓰기 전에는 432였다. 그 차이는 `element(at:)`처럼 class-bound 파라미터를 몰라
      잘못 허용되던 것이고, 이제 거부한다.
    - MusicKit 885/974, WeatherKit 347/382, AppIntents 881/3771.

- graph의 `swiftGenerics` 밖에 있는 정보(2026-11-22, rewrite 수정이라 ABI와 무관):
  - `@preconcurrency`: 선언 조각에 있는데 버리고 있었다. 이제 `@preconcurrency`로 쓴다. 심볼은 function type의
    `@Sendable`과 global actor를 뺀다(`Button.init(action:label:)`, `NewDocumentAction.callAsFunction(_:)`).
  - `where T: AnyObject`: `swiftGenerics`에는 없고 선언의 `where`에만 있다. 거기서 읽는다(`RlzC`).
    `UndoManager.registerUndo(withTarget:handler:)`, `FocusedValue.init(_:)`.
  - 표준 라이브러리 타입 안의 타입(`String.Index`): 바깥 이름을 그냥 쓰면 `String`을 모듈로 읽었다
    (`6String5IndexV`). 이제 `Swift.String(struct).Index(struct)`로 쓴다(`SS5IndexV`).
  - 남은 SwiftUI 차이 5개는 `NSHostingView`의 `@objc override` 속성이다. Swift 심볼이 없으므로 Objective-C(objc2)의
    몫이다.
  - 결과:
    - SwiftUI 1752 → 1774/2956
    - Foundation 3699 → 3733/4905
    - WeatherKit 347 → 353/382
    - StoreKit 629 → 630/658

- Foundation mangling 차이 106개 정리(2026-11-22):
  - `__shared`: graph 선언 조각에는 없고 USR에만 있다(`h`). 쓴 선언의 심볼과 USR이 `h`만 다르면, 하나씩 붙여 보아
    USR에 가까워지는 파라미터에만 `__shared`를 쓴다(빌린다). 최종 심볼은 여전히 USR과 비교한다.
    `URL.init(string:)`, `CharacterSet.init(charactersIn:)` 등 32개.
  - 다른 모듈이 extension으로 선언한 타입(`String.Encoding`, `String.LocalizationValue`): USR이 바깥 타입 USR 뒤에
    `10FoundationE`로 이어지면 `Swift.String(struct).(extension in Foundation) Encoding(struct)`로 쓴다. `0`으로
    시작하는 식별자(`05UnitsE0`, 단어 치환)는 모듈이 아니다.
  - `any Any.Type`: `any Any`/`any AnyObject`를 `Any`/`AnyObject`로 읽는다(전에는 `Foundation.Any` 프로토콜로
    읽었다). corpus `mt10`.
  - 제약 있는 extension 안의 타입(`extension Measurement where UnitType: Dimension { struct AttributedStyle }`):
    graph의 그 타입 `swiftExtension`에서 바깥 타입이 이미 선언한 요구사항을 빼고 context와 시그니처의 타입에 표시한다.
    swiftc spike로 확인: 그런 타입의 init/static은 `T`와 그 extension의 witness table, 인스턴스 멤버는 `Self`
    metadata를 받는다(이미 승인된 규칙대로). `where T == Int`인 extension 안의 타입은 아무것도 받지 않는다.
  - 버그: path의 extension이 `T == Int`로 고정한 타입(`Holder<T>.(extension in M where T == Int) Fixed`)의 멤버를
    apus는 `T`를 요구하고 `Self` metadata/`T` metadata를 넘기려 했다(손으로 쓴 선언에서만 닿았다). 이제
    2026-11-22 규칙대로 고정된 파라미터로 센다.
  - 남은 5개: `Predicate`/`Expression`의 `init(from:)`(parameter pack), `NSHashTableCallBacks` 등 C 함수 포인터 struct
    3개(objc2의 몫). `Measurement…ByteCount`는 path에 제약 extension이 두 개라 parse에서 거부된다.
  - 결과: Foundation 3733 → 3816/4905, SwiftData 431 → 438, AppIntents 881 → 892(차이 615 → 534).

- AppIntents mangling 차이 534개(2026-11-22): 533개가 `_const`(`Yt`, 컴파일 타임 상수 파라미터)를 graph가 빠뜨린
  것이었다. `__shared`와 같은 방법으로 USR에서 되찾는다. `_const`는 호출하는 쪽의 타입 검사 제약일 뿐 전달 방식은
  같다(parser는 이미 받고 있었다). 단, Rust에서 넘긴 값은 빌드 때 추출되는 AppIntents 메타데이터에 들어가지 않는다.
  AppIntents 892 → 1425/3771. 남은 차이 1개(`SyncableEntityIdentifier`).

- AppIntents "unknown type" 1072개(2026-11-22): 990개가 extension `where`의 same-type 요구사항
  (`Value.ValueType == Measurement<UnitTemperature>`)이었다. `swiftGenerics`는 바깥 타입(`Measurement`)의 USR만 주고
  generic 인자는 이름으로만 쓴다. 인자를 바깥 타입의 모듈, 그다음 선언 모듈의 graph에서 이름으로 찾는다
  (`__C.NSUnitTemperature(class)`). 잘못 찾으면 USR과 다른 심볼이 되어 거부된다.
  - 같이 드러난 것: 연달아 나오는 같은 표준 타입은 `S2b`로 줄여 쓰이지만 `_const`가 붙으면 `SbYtSbYt`로 나뉜다. 표시를
    되찾을 때 둘 다 풀어 쓴 뒤 비교한다.
  - 결과: AppIntents 1425 → 2419/3771(mangling 차이 0), SwiftUI 1774 → 1823, Foundation 3816 → 3819, WeatherKit
    353 → 355.
  - SwiftUI에 새로 드러난 차이(거부됨): `CGFloat`가 일부 심볼에서 CoreFoundation으로 mangle됨(2), `Parent == Leaf` 같은
    두 generic 파라미터의 same-type을 대표 파라미터로 바꾸지 않음(2), `ScrollViewReader.content`(1).

- 연관 타입(2026-11-22): by-name rewrite가 `T.Element`, `Self.ID`(USR `…Qa`)를 graph처럼 쓴다. 전체 선언 문법과 mangler는
  이미 지원했고(값은 generic 값처럼 전달), 새 ABI 규칙은 없다. 같이 고친 것: `swiftGenerics`는 확장 중인 protocol에 대한
  conformance(`extension DataProtocol`의 `where D: DataProtocol`)를 빠뜨린다. 선언 자신의 `where`에서 읽는다.
  - 결과: Foundation 3819 → 3876, AppIntents 2419 → 2558, SwiftUI 1823 → 1887, SwiftData 438 → 465(mangling 차이 0),
    MusicKit 885 → 903, StoreKit 630 → 632.
  - 남은 것(거부됨): 파라미터가 protocol 여럿을 따를 때 연관 타입을 선언한 protocol(`(of P)`)을 graph가 주지 않는다(23개;
    표준 라이브러리 protocol은 graph가 없다). 연관 타입이 `where`로 구체 타입이 되는 경우(`Self.NeverResult` → `Never`,
    `Format.FormatInput` → `Decimal`): Swift는 구체 타입으로 mangle하는데 graph는 이름만 준다. `@preconcurrency`로
    `Sendable` 요구사항을 빼면 빈 `rl`이 남는다(`AttributeContainer.Builder.callAsFunction`, 1개).

- 다시 내보내는 모듈(2026-11-22): 요구사항의 generic 인자를 이름으로 찾을 때 그 모듈이 `@_exported import`하는 모듈도
  본다. SwiftUI의 `Text`/`Image`/`View`는 `SwiftUICore`에 있다(mangle은 `@_originallyDefinedIn`으로 SwiftUI).
  SwiftUI 1887 → 1919. `Label<Text, Image>` 꼴 92개 중 나머지 대부분은 이제 "export 안 됨"(inlinable)으로 분류된다.

- "class-bound generic 값은 파라미터나 결과 자체로만" 거부 143개(2026-11-22, `APUS_COVERAGE_DETAIL`이 이제 `REFUSED`도
  출력한다): SwiftUI 135개는 closure 안(그중 128개가 `TableColumn.init`의 `(RowValue) -> Content`, `RowValue: NSObject`),
  SwiftData 7개는 optional(`ModelActor.subscript` 등), 1개는 closure. closure/optional 안의 lowering은 관찰된 것이라
  소유자 결정이 필요하다.

## 남은 큰 항목

- StoreKit mangling 차이 20개(`SKError`: clang이 만든 error struct).
- Foundation "export 안 됨" 660개 분석(2026-11-21, `.swiftinterface`와 `.tbd`로 확인):
  - 217개: importer가 클라이언트마다 만드는 C 타입 멤버(`NS_OPTIONS`/`NS_ENUM`의 `init(rawValue:)`, struct의 memberwise
    `init`, typed string의 `init(_: String)`). 심볼이 원래 없다. Rust에서는 objc2 값을 직접 만든다. coverage는 이제
    Objective-C/C 선언과 함께 제외한다(Swift extension 밖의 `__C` 멤버). Foundation 3512/4902, StoreKit 629/658.
  - 391개: `Locale.LanguageCode`/`Region`/`Script` 등의 `@_alwaysEmitIntoClient` 상수. `.tbd`에는 property descriptor(`MV`)만
    있다.
  - 나머지 약 50개도 `@_alwaysEmitIntoClient`/`@inlinable`이다: `FormatStyle.percent` 같은 static 멤버, `URLSession`의 async
    편의 메서드, `Data.span`/`append(_: UInt8)`, `AttributedString` view의 `count`/`index`, `TimeZone.gmt`. shim 없이는 부를 수
    없다.
  - 2개만 mangling 문제다: `URL.init(_: FilePath)`, `init(_:isDirectory:)`. 이 extension에 `@_originallyDefinedIn(module:
    "_System_Foundation", macOS 15)`이 붙어 있어 swiftc는 `$s10Foundation3URLV08_System_A0EyACSg0C08FilePathVcfC`
    (`(extension in _System_Foundation)`)를 참조하는데, graph USR은 extension 표시가 없다. 지금은 `.tbd` 검사로 컴파일 오류가
    나므로 안전하다. Foundation 전체에서 이런 extension은 두 개(`URL`, `FilePath`)뿐이라 미뤄 둔다.

- 프로토콜의 associated type(`…Qa`, `Self.Filter`)과 stdlib typealias.
- export되지 않은 것(`@inlinable` 등): shim이 필요하다.
- AppIntents: result builder와 string interpolation 같은 컴파일 시점 기능, 그리고 "parse: expected …".
- 제네릭 builder를 지운 뒤 새로 파싱된 선언 중 SwiftUI `Table.init` 19개와 AppIntents 2개는 심볼이 다르다(거부됨):
  같은 타입 요구사항(`Value == Columns.TableRowValue`)의 정규화와 여러 프로토콜을 가진 파라미터의 associated type(`(of P)`).
