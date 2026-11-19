# 사용 편의 개선: Swift 모듈을 쓰는 Rust 프로그램의 준비 작업 줄이기

## 왜

`examples/swift-data`를 만들어 보니 apus를 실제로 쓰기 전에 필요한 준비가 많다. Swift 모델을 빌드하는 build.rs가 25줄이고,
mangled 이름 네 개(`"5Todos4TodoC"`, `"$s9SwiftData15PersistentModelMp"` 등)를 손으로 써야 한다. 클래스 init에는 직접 구한
`self: &SwiftType`을 넘겨야 하고, 제네릭 메서드 `fetch` 하나에는 `SwiftFn`과 `"Tj"` 접미사, `call_returning`, 결과 타입,
witness table을 모두 손으로 다뤄야 한다. 목표는 이런 준비를 Swift 선언 문법으로 쓰는 한 줄 수준까지 줄이는 것이다.

## 목표 / 비목표

- 목표:
  - build.rs를 한 줄로 줄인다.
  - mangled 이름을 손으로 쓸 일을 없앤다.
  - 클래스 init과 static 멤버를 `self` 없이 호출할 수 있게 한다.
  - 제네릭 파라미터를 구체 타입으로 고정해 `fetch`를 일반 `swift!` 선언으로 쓸 수 있게 한다.
  - rust-analyzer가 터미널 빌드와 충돌하지 않게 한다.
- 비목표:
  - SwiftPM 패키지 빌드 지원.
  - `cargo apus init` 같은 템플릿.
  - macOS 외 대상(iOS 등)에서 Swift 소스를 빌드하는 것.
  - 사용자 프로젝트의 `MACOSX_DEPLOYMENT_TARGET` 설정을 없애는 것. 바이너리 자체가 14.0을 목표로 해야 하고, 의존 crate는 이를 대신
    설정할 수 없다. 대신 apus-build가 같은 안내 메시지를 낸다.
- 반목표(이렇게 되면 실패):
  - ABI가 보장하지 않는 경로를 새로 쓰는 것.
  - 기존 선언의 의미나 오류 검사를 약하게 만드는 것.

## 제약 (ABI)

새로 쓰는 경로는 모두 이미 쓰고 있는 문서화된 것이다.

- **타입 이름:** `docs/ABI/Mangling.rst`로 만든 이름을 `swift_getTypeByMangledNameInContext`로 찾는다. 기존 `result_type!`과
  같다.
- **프로토콜:** 프로토콜 descriptor 기호(`…Mp`)와 `swift_conformsToProtocol`. 기존 자동 제네릭 인자와 같다.
- **호출 규약:** 클래스 init/static의 `self`는 클래스 metadata이다(swiftself). 제네릭 인자는 형식 인자 뒤에 metadata,
  그다음 witness table 순서로 간다. 둘 다 기존 `self: &SwiftType` 경로와 자동 제네릭 경로가 이미 쓰는 규약이다.

새로 승인받을 규칙은 없다.

## 접근 방식

검토한 방식:
- **A. 매크로와 빌드 도우미로 줄이기(선택).** 위 다섯 가지를 모두 하는 방식이다.
- **B. SwiftPM 패키지를 빌드하는 도우미.** Swift 쪽을 패키지로 둘 수 있지만, 링크 설정과 산출물 위치가 복잡해 효과에 비해 일이 크다.
- **C. `cargo apus init` 템플릿.** 처음 설정만 줄어들 뿐 코드 안의 mangled 이름과 수동 제네릭은 그대로 남는다.

소유자가 A의 1~5번을 모두 진행하기로 했다(2026-11-20).

## 설계

### 1. `apus-build` crate (`builder/`, workspace 멤버)

```rust
// build.rs
fn main() {
    apus_build::SwiftModule::new("Todos", "swift").build();
}
```

- `sources`는 디렉터리 하나(그 안의 `*.swift` 전부, 정렬된 순서) 또는 파일 하나다. `rerun-if-changed`를 내보낸다.
- swiftc는 `xcrun swiftc -parse-as-library -static -emit-library -enable-library-evolution -module-name {name}`로
  부른다. 그 밖의 플래그는 다음과 같다.
  - `-O`: 프로필이 release일 때만 붙인다. 디버그 빌드에서는 `-Onone`(제 결정: 디버그 빌드가 빨라야 해서).
  - `-target {arch}-apple-macos{MACOSX_DEPLOYMENT_TARGET}`: arch는 `CARGO_CFG_TARGET_ARCH`에서 가져오고, `aarch64`는
    `arm64`로 바꾼다.
- 결과는 `OUT_DIR/lib{name}.a`이다. `rustc-link-search`와 `rustc-link-lib=static=…`를 내보내고, toolchain의 compat 라이브러리
  경로(`swiftc/../../lib/swift/macosx`)도 넣는다.
- library evolution은 기본으로 켠다. Apple 프레임워크와 같은 방식으로 컴파일되어야 클래스 멤버 호출이 같은 경로를 탄다.
  `.library_evolution(false)`로 끌 수 있다. `.flag("…")`로 swiftc 플래그를 더할 수 있다.
- 다음 경우에는 메시지와 함께 panic한다.
  - `MACOSX_DEPLOYMENT_TARGET`이 없다. 메시지는 apus-sys와 같이 `.cargo/config.toml`의 `[env]`를 안내한다.
  - 대상이 macOS가 아니다.
  - swiftc가 실패한다.
- 의존성은 없다(std만).

### 2. `swift_type!` / `swift_protocol!`

- `apus::swift_type!("Todos.Todo(class)")`는 `&'static str` mangled 이름(`"5Todos4TodoC"`)으로 펼쳐진다.
  - `SwiftType::from_mangled_name(apus::swift_type!(…))`로 쓴다.
  - 제네릭 인자도 된다: `"SwiftData.FetchResultsCollection(struct)<Todos.Todo(class)>"`.
  - 이름 앞에 붙은 모듈이 타입 이름의 기본 모듈이다. 선언문의 bare 이름 규칙과 같다.
- `apus::swift_protocol!("SwiftData.PersistentModel")`는 프로토콜 descriptor 기호(`"$s9SwiftData15PersistentModelMp"`)로
  펼쳐진다. `SwiftProtocol::new(apus::swift_protocol!(…))`로 쓴다.
- mangle crate에는 `type_name(text)`와 `protocol_descriptor_of(text)`를 더한다.
  - `type_name`은 내부적으로 `{module}.__t() -> {type}` 선언을 파싱해 기존 `result_type` 경로로 처리한다.
  - 둘 다 corpus 테스트로 swiftc가 내는 이름과 대조한다.
- 제 결정: 문자열 대신 `SwiftType`을 바로 돌려주는 편의 형태는 만들지 않는다. 런타임 조회의 실패(`Option`)가 호출 지점에서 보여야 해서,
  기존 `from_mangled_name`을 그대로 쓴다.

### 3. 클래스 init·static 멤버에서 `self` 생략

- 지금은 `self: &SwiftType`을 쓰지 않으면 컴파일 오류다. 앞으로는 생략할 수 있다.
  - 생략하면 선언된 클래스의 metadata가 `self`가 된다. 그 metadata는 선언의 문맥 타입 이름(mangled)으로 조회하고, 함수마다
    `static`에 한 번만 저장한다.
  - 찾지 못하면 호출이 `CallError::Unavailable`을 돌려준다.
- `self: &SwiftType`을 명시하는 기존 방식도 그대로 된다. 하위 클래스의 metadata를 넘기는 경우가 그렇다.
- 문맥 타입이 제네릭이면(`Box<T>`) 이름이 하나로 정해지지 않으므로 생략할 수 없다. 기존 오류를 유지하되, 메시지에
  `self: &SwiftType`이 필요한 이유를 적는다.
- 구현: `apus_mangle::Declaration::context_type_name()`과 `__swift_route`의 `classself "name"` 표시, 그리고 receiver 자리에
  들어가는 `DeclaredClass` 값.

### 4. 제네릭 파라미터 구체 지정

```rust
#[swift = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>, batchSize: Int) \
           throws -> FetchResultsCollection(struct)<T>", T = "Todos.Todo(class)"]
fn fetch_todos(self: &SwiftObject, descriptor: &SwiftValue, batch: i64) throws -> SwiftValue;
```

- 속성 안에 `#[swift = "…", T = "타입"]` 형태로 쓴다. `swift_linked`도 같다. 제 결정: 별도 속성보다 기존 매크로 패턴과 잘
  맞고 짧다.
- 컴파일 시점에 mangle crate의 `Declaration::bind(&[("T", "Todos.Todo(class)")])`가 계획을 만든다.
  - 제네릭 파라미터마다 구체 타입의 mangled 이름.
  - witness table: 정규 순서의 (파라미터, 프로토콜 descriptor, layout).
  - 결과 타입: T를 치환한 구체 이름(`9SwiftData22FetchResultsCollectionVy5Todos4TodoCG`).
- 런타임에는 타입 이름을 조회하고, 그 결과로 conformance를 조회한다. 둘 다 기존 `GenericArgs`로 넘긴다. 하나라도 실패하면
  `CallError::Unavailable`, 프로토콜 layout이 맞지 않으면 기존과 같은 `InvalidArgument`이다.
- 다음은 컴파일 오류다.
  - 모든 제네릭 파라미터가 지정되지 않았다.
  - 선언에 없는 이름이 쓰였다.
  - 자동 계획이 거절하는 경우. 연관 타입 요구, 한 파라미터에 프로토콜 둘 이상, ObjC 프로토콜, 제네릭 타입의 제네릭 멤버가
    해당하며, 기존 메시지와 같다.
  - 구체 타입을 mangle할 수 없다.
- 구체 지정을 하면 Rust 시그니처에는 형식 인자만 쓴다. 자동 모드와 같다. class-bound 파라미터 값은 여전히 `&SwiftObject`이다.

### 5. `.vscode/settings.json`

```json
{
    "rust-analyzer.cargo.targetDir": true,
    "rust-analyzer.cargo.features": ["dispatch", "test-probe", "stream", "block2"]
}
```

rust-analyzer가 자기 target 디렉터리를 쓰게 해서, 터미널 cargo가 proc-macro dylib을 다시 쓰는 도중에 읽다가 실패하는 일을 막는다.
테스트 feature도 켜서 rust-analyzer가 테스트 파일을 분석하게 한다.

### 예제와 문서

- `examples/swift-data`를 새 방식으로 다시 쓴다.
  - build.rs는 한 줄이 된다.
  - mangled 이름과 `SwiftFn`, `call_returning`이 없어진다.
  - 실행 결과는 지금과 같아야 한다.
- README:
  - "Using a Swift module of your own"(apus-build) 절을 새로 둔다.
  - `swift!` 절에 self 생략과 구체 지정을 적는다.
  - 매크로 목록에 `swift_type!`/`swift_protocol!`을 넣는다.
- CLAUDE.md 레이아웃 표에 `builder/`를 넣는다.

## 테스트 (적게, RED 먼저)

- **mangle:**
  - `type_name`/`protocol_descriptor_of`의 corpus 대조.
  - `bind`의 계획을 단위 테스트한다(이름, witness 순서, 결과 이름, 오류).
- **macros(compile_fail doctest):**
  - 구체 지정에서 빠진 파라미터, 없는 이름.
  - 제네릭 클래스 init에서 self를 생략한 경우.
- **런타임(probe):**
  - `Res`의 클래스 init을 self 없이 호출한다.
  - 구체 지정한 제네릭 호출 하나(예: `Res.same<T: AnyObject>`, 혹은 새 probe).
- **apus-build:** 예제 crate가 이것으로 빌드되고 실행되는 것으로 확인한다. 별도 테스트는 두지 않는다(제 결정).

## 빌더에게 맡기는 것

- 위에 적은 표시(`classself`, `concrete [...]`)의 정확한 토큰 모양.
- `DeclaredClass` 같은 숨은 타입의 이름.
- probe 선언의 세부.
