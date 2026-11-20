# symbol graph로 선언 쓰기 줄이기 (B: kind 생략, C: 이름만으로 선언)

## 왜

`swift!` 선언에는 타입마다 `(class)`, `(struct)`, `(AnyObject)`를 손으로 적어야 하고, 시그니처 전체를 Swift 인터페이스에서 옮겨 적어야
한다. 리서치(2026-11-20)에서 Xcode의 `swift-symbolgraph-extract`가 컴파일러가 검사한 선언, kind, 제약을 준다는 것을 확인했다. 소유자가
A(`.tbd` 검사, `fe51b02`)에 이어 B와 C를 진행하기로 했다.

## 목표 / 비목표

- 목표:
  - **B.** 전체 선언에서 kind 표기를 생략할 수 있다: `SwiftData.ModelContext.fetch<T: PersistentModel>(_: FetchDescriptor<T>) throws -> [T]`.
  - **C.** 이름과 레이블만 적을 수 있다: `SwiftData.ModelContext.fetch(_:)`, `Todos.Todo.title { get }`, `Todos.Todo.init(title:priority:)`.
    시그니처는 symbol graph에서 가져온다.
  - 지금처럼 kind를 다 적은 선언은 그대로 동작하고, 결과도 같다.
- 비목표:
  - 오버로드 자동 선택. 같은 이름과 레이블의 선언이 둘 이상이면 오류를 내고 전체 선언을 쓰게 한다.
  - 제약 있는 extension의 멤버, 파라미터 팩, ObjC 외의 C 타입을 C에서 자동으로 옮기는 것. 이런 경우는 오류로 전체 선언을 요구한다.
- 반목표:
  - USR을 호출 기호로 쓰는 것(ABI 문서에 없는 관찰이다).
  - 검증 없이 재구성한 선언을 쓰는 것.

## 제약 (ABI)

- 호출 기호는 지금처럼 apus-mangle이 선언에서 만든다(Mangling.rst).
- symbol graph는 선언의 텍스트와 kind를 얻는 데만 쓴다. USR은 어떤 선언인지 고르고 교차검증하는 데만 쓴다.
- C로 만든 선언은 두 번 검증한다. 하나라도 어긋나면 컴파일 오류다.
  1. apus가 만든 기호가 USR에서 유도한 기호와 같아야 한다(`s:`→`$s`, init `fc`→`fC`, 프로퍼티 `vp`→`vg`/`vs`, subscript `ip`→`ig`/`is`).
  2. SDK 모듈이면 그 기호가 `.tbd`에 있어야 한다(A).
- 새로 승인받을 규칙은 없다.

## 설계

### symbol graph 얻기

- **SDK 모듈:** `xcrun swift-symbolgraph-extract -module-name M -target <arch>-apple-macosx<SDK 버전> -sdk <SDK>`.
  - 결과를 `$TMPDIR/apus-symbolgraphs/<SDK 경로와 버전의 해시>/`에 캐시한다. 처음 한 번은 모듈당 약 4초가 걸린다.
  - 임시 파일에 쓴 뒤 rename해서, 여러 rustc 프로세스가 동시에 써도 안전하게 한다.
- **프로그램 자체 모듈:** apus-build가 `-emit-symbol-graph`로 `OUT_DIR`에 graph를 만들고 `cargo:rustc-env=APUS_SYMBOL_GRAPHS=<dir>`를
  내보낸다. proc macro는 이 디렉터리를 먼저 찾는다. 테스트 probe(`Res`)도 같은 방식이다.
- **표준 라이브러리와 `_Concurrency`:** graph를 뽑지 않는다. apus가 아는 이름 표를 그대로 쓴다.
- JSON 파싱은 `serde_json`(apus-macros의 새 의존성)으로 하고, proc macro 프로세스 안에 캐시한다.

### B: kind 생략

- apus-mangle에 `KindResolver` trait과 `Declaration::parse_resolving(text, &resolver)`를 둔다.
- parser는 kind가 없는 이름에서만 resolver를 부른다. 묻는 것은 (모듈, 경로) → kind와, 프로토콜이 class-bound인지다.
- class-bound 판단:
  - 프로토콜 선언의 상속 목록에 `AnyObject`가 있으면 class-bound다.
  - 상속한 프로토콜이 class-bound여도 class-bound다(재귀).
- bare 이름은 지금처럼 선언의 모듈에 속한다. 다른 모듈의 타입은 `Module.Name`으로 쓴다.
- proc macro는 모두 같은 도우미 `declaration(text)`로 파싱한다. 대상은 `__swift_route`, `__call_symbol`, `__dispatch_thunk`, 결과 타입,
  `mangle!` 등이다.

### C: 이름만

- **짧은 형태:**
  - `Module.Type.name(label:label:)`
  - `Module.Type.name { get }` / `{ set }`
  - `Module.Type.init(label:)`, `Module.Type.subscript(_:) { get }`
  - `static`은 symbol graph가 알려 준다.
- **재구성:** symbol graph의 `declarationFragments`와 `swiftGenerics`로 apus 문법의 전체 선언을 만든다.
  - 문맥 타입과 파라미터·결과 타입에 모듈과 kind를 붙인다. kind는 USR 끝 글자로 정한다.
  - `where T : P` 제약은 `<T: P(AnyObject)>`로 옮긴다.
  - `mutating`, `@MainActor`, `nonisolated`, `async`, `throws`는 유지한다.
  - 버려도 의미가 바뀌지 않는 표기만 버린다: `@discardableResult`, `final`, `open`, `override`, `required`, `convenience`.
  - 그 밖의 표기나 옮길 수 없는 형태가 나오면 오류를 낸다. 오류에는 재구성한 선언을 함께 보여 준다.
- 재구성한 선언은 위 두 검증을 거친 뒤에야 쓴다.

## 테스트 (적게, RED 먼저)

- B:
  - `Res` 선언 하나를 kind 없이 쓴다.
  - SwiftData 선언 하나(`PersistentModel`의 class-bound 자동 판단 포함).
- C:
  - `Res` 메서드, 프로퍼티, init 하나씩.
  - SwiftData `fetch(_:)` 하나.
  - 오버로드가 모호한 경우와 없는 이름을 쓴 경우의 compile_fail 하나씩.
- SwiftData 예제를 짧은 형태로 바꾼다. 실행 결과가 같아야 한다.
