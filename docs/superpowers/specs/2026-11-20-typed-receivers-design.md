# 타입 있는 receiver: 잘못된 객체로 멤버를 부르는 UB 막기

## 왜

`SwiftObject`/`SwiftValue`는 타입이 지워진 그릇이라, `title(&container)`처럼 다른 타입의 값을 receiver로 넘겨도 컴파일되고
실행하면 UB이다. 소유자가 두 가지를 함께 하기로 했다(2026-11-20, 선택지 (a)).

1. 타입이 지워진 receiver는 호출마다 실행 시점에 검사한다.
2. 타입 있는 wrapper(`class Todo = "Todos.Todo(class)";`)와 `impl` 블록을 두어, 컴파일 시점에 검사하고 메서드 문법으로 부른다.

## 목표 / 비목표

- 목표:
  - 선언이 정한 타입이 아닌 receiver로는 Swift 코드가 실행되지 않는다(`CallError::InvalidArgument`).
  - wrapper를 쓰면 receiver, 인자, 결과의 타입이 컴파일 시점에 맞는지 확인되고 실행 시점 비용이 없다.
  - `todo.title()` 같은 메서드 문법을 쓴다.
  - `-> Vec<Todo>`로 배열 결과를 wrapper 목록으로 받는다.
- 비목표:
  - 프로토콜 타입(`any P`) wrapper.
  - 제네릭 wrapper(`Box<T>`를 Rust 제네릭으로 표현하는 것).
  - `#[symbol]`/`#[linked]`(선언 없는 기호) 호출의 검사. 선언이 없어서 타입을 모른다.
- 반목표:
  - ABI가 보장하지 않는 경로를 새로 쓰는 것.
  - 올바른 호출을 거부하는 것. 하위 클래스 객체는 상위 클래스의 멤버를 부를 수 있어야 한다.

## 제약 (ABI)

새로 쓰는 것은 모두 런타임이 export하는 진입점이다.

- `swift_dynamicCastUnknownClass(object, targetType)`(include/swift/Runtime/Casting.h): 객체가 그 클래스이거나 하위 클래스인지
  확인한다. Swift 클래스와 ObjC 클래스를 모두 다룬다.
- `swift_getTypeName`(include/swift/Runtime/Metadata.h; 이미 `SwiftType::name`이 쓴다): 제네릭 값 타입의 검사에 쓴다.
- 구체 타입의 metadata는 런타임이 하나만 만든다(uniqued). 그래서 포인터가 같은지로 비교할 수 있다.

새로 승인받을 규칙은 없다.

## 설계

### 1. 실행 시점 receiver 검사

Rust receiver가 `&SwiftObject`, `&SwiftValue` 또는 `&mut SwiftValue`이고 선언이 있을 때(`#[swift]`, `#[swift_linked]`) 다음과 같이
검사한다.

| 선언의 문맥 | 검사 |
|---|---|
| 구체 클래스(`Todos.Todo(class)`) | `swift_dynamicCastUnknownClass`. 하위 클래스도 통과한다 |
| 구체 struct/enum | 값의 metadata가 선언 타입의 metadata와 같아야 한다 |
| 제네릭 struct/enum(`FetchResultsCollection<Element>`) | 값의 타입 이름에서 제네릭 인자를 지운 것이 선언의 타입 이름(`SwiftData.FetchResultsCollection`)과 같아야 한다 |
| 제네릭 클래스, 프로토콜 | 검사하지 않는다(README에 적는다) |

- 선언 타입의 metadata는 이름으로 한 번 찾아서 함수마다 `static`에 둔다. 찾지 못하면 `CallError::Unavailable`이다.
- 실패하면 `CallError::InvalidArgument("the receiver is not of the declaration's type")`이다.
- 제 결정: 제네릭 값 타입은 이름으로 비교한다. metadata의 `Description` 필드는 arm64e에서 서명된 포인터라 직접 읽지 않는다. 이름 비교는
  잘못 통과시킬 수는 없고(이름이 같으면 같은 nominal 타입), 잘못 거부할 수만 있다.

### 2. wrapper 선언

```rust
apus::swift! {
    pub class Todo = "Todos.Todo(class)";
    pub struct Query = "SwiftData.FetchDescriptor(struct)<Todos.Todo(class)>";   // enum도 같은 형태
}
```

- 클래스는 `#[repr(transparent)] pub struct Todo(SwiftObject)`, 값 타입은 `pub struct Query(SwiftValue)`가 된다.
  `Clone`, `Deref<Target = SwiftObject | SwiftValue>`, `into_inner()`, `swift_type()`을 갖는다.
- 타입이 지워진 값에서는 `Todo::try_from(object)`로 만든다. 실행 시점에 위 1과 같은 방식(클래스는 dynamic cast, 값 타입은 metadata
  비교)으로 검사하고, 실패하면 `CallError::InvalidArgument`이다.
- 숨은 trait `SwiftWrapper`가 mangled 이름과 nominal 이름(`Todos.Todo`)을 상수로 갖는다. receiver, 인자, 결과 trait은 apus가
  `SwiftWrapper`에 대한 blanket impl로 제공한다.

### 3. 컴파일 시점 검사

route가 receiver, 각 파라미터, 결과에 대해 다음 조건에서 `const { … }` 검사를 넣는다.

- 조건:
  - Swift 쪽 타입이 사용자 nominal 타입이다(문맥 타입, 파라미터, 결과. `Optional`과 `[…]`은 벗긴다).
  - Rust 쪽 타입이 타입이 지워진 그릇(`SwiftObject`, `SwiftValue`, `DirectValue`, `Retained`, …)이 아니다.
- 비교:
  - 구체 타입이면 mangled 이름이 같아야 한다.
  - 문맥이 제네릭이면 nominal 이름이 같아야 한다.
- 다르면 컴파일 오류이고, wrapper가 아닌 타입이면 trait 미구현 오류이다.

### 4. `impl` 블록

```rust
apus::swift! {
    impl Todo {
        #[swift_linked = "Todos.Todo(class).title: String { get }"]
        pub fn title(&self) -> String;
    }
}
```

- `&self`/`&mut self` 메서드와 associated function(self 없음)을 받는다. receiver는 `Self`이므로 3의 검사를 받는다.

### 5. `Vec<Wrapper>` 결과

- `[Todo]` 결과를 `-> Vec<Todo>`로 받는다.
- 배열은 `SwiftIter`(문서화된 `Sequence` 경로)로 순회한다. 원소가 클래스이면 ObjC 배열일 수 있어서, 기존 `Vec<T>` 변환처럼 내부 배치를 읽지
  않는다.
- 원소는 Swift 선언이 보장하므로 다시 검사하지 않는다.

## 테스트 (적게, RED 먼저)

- receiver 검사:
  - 다른 클래스의 객체, 다른 struct 값, 다른 제네릭 struct 값을 넘기면 `InvalidArgument`이다.
  - 하위 클래스 객체는 통과한다.
- wrapper:
  - `try_from` 성공과 실패.
  - 메서드 호출.
  - `Vec<Wrapper>` 결과.
- compile_fail doctest: 다른 wrapper를 receiver나 인자로 넘기는 경우.
- SwiftData 예제를 wrapper로 바꾸고, 실행 결과가 같아야 한다.
