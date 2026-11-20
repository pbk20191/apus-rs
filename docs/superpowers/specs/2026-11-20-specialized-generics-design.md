# 제네릭은 특수화해서 부른다 (bindings 확장)

## 왜

이름만으로 쓴 선언(`SwiftData.ModelContext.fetch(_:)`)과 제네릭 함수의 조합은 자동 추론으로 다루기 어렵다. 소유자가
(2026-11-20) "제네릭 함수는 쓰는 곳마다 특수화하게 하자"고 정했다. 모든 제네릭 파라미터를 구체 타입에 묶으면 호출 하나가 구체
함수 하나가 되고, 지금 거부하는 경우 대부분이 이름으로 찾고 실행 시점에 검사하는 문제로 바뀐다.

## 목표 / 비목표

- 목표:
  - bindings(`T = "…"`)로 다음을 부를 수 있다.
    1. 제네릭 타입의 멤버. 타입의 파라미터도 묶는다(`Box<T>.map<U>`에 `T`, `U`).
    2. 한 파라미터에 프로토콜이 여럿인 경우(`T: Hashable & Comparable`).
    3. superclass 제약(`T: Animal`). 묶은 클래스가 그 클래스나 하위 클래스인지 실행 시점에 검사한다.
    4. 제네릭 에러 타입의 typed throws(`throws(E)`, `E`가 파라미터 자체일 때).
    5. associated type 요구사항. 같음(`S.Element == String`)은 실행 시점에 검사하고, 준수(`S.Element: P`)는 witness table을
       찾아 넘긴다.
  - 이름만으로 쓴 제네릭 선언은 모든 제네릭 파라미터를 묶어야 한다(자동 추론을 쓰지 않는다).
  - 검사가 실패하면 Swift 코드는 실행되지 않고 `CallError`가 된다.
- 비목표:
  - 프로토콜 멤버의 `Self`를 묶는 것. 프로토콜 멤버는 지금처럼 `Opened`로 부른다.
  - 묶지 않은 선언의 자동 추론 범위를 넓히는 것.
  - 파라미터를 구체 타입으로 고정하는 extension(`where T == Int`) 안의 static 멤버와 initializer. 지금처럼 거부한다.
  - 파라미터 팩.
- 반목표:
  - 묶은 타입이 선언의 제약을 어기는데 호출되는 것.

## 제약 (ABI)

실험(2026-11-20, `swiftc -enable-library-evolution -emit-ir`)으로 확인한 인자 순서:

| 선언 | IR 인자 |
|---|---|
| `Box<T>.map<U: P>(_: U)` (instance) | `u, Box<T> metadata, U, U.P, self` |
| `static Box<T>.make<U: P>(_: U)` | `u, T, U, U.P` |
| `PBox<T: P>.init(t:)` | `…, T, T.P` |
| `two<T: Hashable & Comparable>(_: T)` | `t, T, T.Comparable, T.Hashable` (심볼 `xSLRzSHRzl`의 순서) |
| `assoc<S: Sequence>(_: S) where S.Element == String` | `s, S, S.Sequence` |
| `assocP<S: Sequence>(_: S) where S.Element: P` | `s, S, S.Sequence, S.Element.P` |
| `sup<T: Animal>(_: T)` | `t, T` |
| `GBox<T>.map<U: P>(_: U)` (class instance) | `u, U, U.P, self` |
| `extension Box where T: P { func q() }` | `Box<T> metadata, T.P, self` |
| `extension GBox where T: P { func q() }` | `T.P, self` |

- 1~5는 이미 쓰는 규칙만으로 된다.
  - 제네릭 인자는 metadata 다음 witness table이고, 정규 요구사항 순서를 따른다. 이 순서는 심볼에 적힌 순서이다(Mangling.rst).
  - generic struct/enum의 instance 멤버는 `Self` metadata를 받는다(2026-11-18 승인).
  - static 멤버와 initializer는 타입의 제네릭 인자를 받는다(2026-11-18 승인).
- 새로 쓰는 런타임 진입점:
  - `swift_dynamicCastMetatype`(include/swift/Runtime/Casting.h): superclass 검사.
  - `swift_getAssociatedTypeWitness`와 `swift_conformsToProtocol`: 이미 쓰고 있다.
  - associated type은 프로토콜의 `…TL`/`…Tl` 기호로 찾는다. library evolution 모듈이 export하는 기호이다.
- 프로토콜이 여럿일 때: apus는 선언에 적힌 요구사항으로 심볼을 만든다. Swift가 줄인 요구사항 목록과 다르면 기호가 없어서
  `CallError::Unavailable`(또는 링크 오류)가 된다. 그래서 심볼을 찾았다면 witness table 목록과 순서도 맞다.
- **승인이 필요한 것(이번에는 하지 않는다):** 제약 있는 extension의 instance 멤버는 `Self` metadata(클래스는 isa)에 없는
  witness table만 따로 받는다(위 표 마지막 두 줄). 이것은 관찰된 fulfillment 규칙이라 문서에 없다. 소유자 승인 전까지 거부한다.

## 설계

### bindings

- 선언의 모든 제네릭 파라미터를 이름으로 묶어야 한다. 타입의 파라미터(`Box<T>`의 `T`)와 멤버 자신의 파라미터가 모두 대상이다.
  - 하나라도 빠지면 컴파일 오류이다.
  - 같은 이름이 두 번 나오면(멤버가 타입의 `T`를 가리는 경우) 컴파일 오류이다.
- parser가 문맥 타입의 파라미터 이름을 `Decl.context_names`에 남긴다.
- `Declaration::bind`가 돌려주는 `ConcretePlan`:
  - `types`: 넘길 metadata 순서대로의 타입 이름.
  - `witnesses`: 정규 순서로, 무엇의 witness table인지.
    - 파라미터 자신: `Param(i)`.
    - associated type: `Member(i, protocol의 TL, Tl)`.
  - `checks`: 실행 시점 검사 목록. superclass `(i, class 이름)`과 associated type 같음 `(i, TL, Tl, 기대 타입 이름)`.
  - `error_slot`: typed throws 에러 타입의 파라미터 번호.
  - `self_type`: 묶인 문맥 타입의 이름(`5Probe3BoxVySiG`). receiver 검사와 클래스 static/init의 `self`에 쓴다.

### 문맥별 호출

| 문맥 | 넘기는 제네릭 인자 | receiver / self |
|---|---|---|
| 자유 함수 | 모든 파라미터 metadata, witness | 없음 |
| generic struct/enum instance | 지금처럼 receiver의 `Self` metadata, 그 뒤 멤버 자신의 파라미터 metadata와 witness | receiver는 묶인 타입과 정확히 같아야 한다 |
| generic struct/enum static/init | 타입과 멤버의 모든 파라미터 metadata, 그 뒤 모든 요구사항의 witness(정규 순서) | 없음 |
| generic class instance | 멤버 자신의 것만 | receiver는 묶인 클래스이거나 그 하위 클래스여야 한다(dynamic cast) |
| generic class static/init | 멤버 자신의 것만 | `self`는 묶인 클래스의 metadata(이름으로 찾음) |
| 프로토콜 멤버 | 묶을 수 없다(컴파일 오류) | |

- 인자와 결과 타입에는 묶은 타입을 대입한다. 그러면 wrapper 검사와 결과 타입이 모두 구체 타입이 된다.
- 제약 있는 extension에서 타입의 파라미터에 준수를 더하는 instance 멤버는 거부한다(위 승인 항목).
  - **버그 수정:** 지금은 generic struct/enum만 거부하고, generic class와 프로토콜 extension은 통과시킨다. 이때 witness table 없이
    호출되므로 UB이다. 셋 다 거부한다.

### 실행 시점 검사

- 모든 검사는 호출 전에 한다. 기호나 타입이 없으면 `Unavailable`이다.
  - 준수하지 않으면 `NotConforming`이다.
  - superclass나 associated type이 맞지 않으면 `InvalidArgument`이다.
- 찾은 metadata와 witness는 지금처럼 호출 지점의 `static`에 한 번만 캐시한다.

### 이름만으로 쓴 선언

- 짧은 형태로 쓴 선언이 제네릭이면(문맥 타입 포함) 모든 파라미터를 묶어야 한다. 묶지 않으면 컴파일 오류이고, 오류에 파라미터
  이름을 보여 준다.
- 기존 테스트 `Res.renamed(_:suffix:)`는 `T = "Res.Dog"`를 적는다.

## 테스트 (적게, RED 먼저)

- `Res`에 probe를 추가한다.
  - `Box<T>`: instance 멤버, static 멤버, init, 자신의 제네릭 파라미터.
  - `GBox<T>` 클래스.
  - `two<T: Hashable & Comparable>`, `assoc`/`assocP`, `throws(E)` 함수.
- 통합 테스트:
  - 위 각 형태를 한 번씩 부른다.
  - 실패 경로 셋: superclass 위반, associated type 불일치, 다른 특수화의 receiver.
- mangle 단위 테스트: `bind`가 만드는 `ConcretePlan`(순서와 검사 목록).
- 버그 수정: generic class의 제약 extension 멤버 거부(mangle 단위 테스트, RED 먼저).
- compile_fail: 이름만 쓴 제네릭 선언에 bindings가 없는 경우.
