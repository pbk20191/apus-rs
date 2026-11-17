# objc2 feature: Swift 클래스 참조와 objc2 객체

작성일: 2026-11-17 · 상태: 설계 (승인 대기) · 범위: 소유자가 고른 A (객체 참조만)

## 의도와 목표

Rust가 주인인 앱은 AppKit/Foundation 객체를 objc2 계열(`Retained<NSView>` 등)로 다룬다. 선택 feature `objc2`로 그 객체를
Swift 전용 API의 인자, 반환값, `self`로 직접 쓰고, apus의 `SwiftObject`와 서로 바꿀 수 있게 한다.
예: SwiftUI의 `NSHostingView(rootView:)`를 불러 `Retained<NSView>`로 받아 AppKit 계층에 넣는다.

비목표: 값 브리징(`String`↔`NSString` 등, 범위 B), `@objc` 프로토콜(범위 C), objc2 쪽 클래스 정의(`define_class!`) 연동.

## 먼저 고칠 것: SwiftObject의 참조 카운트

`SwiftObject`는 지금 `swift_retain`/`swift_release`를 쓴다. 이 함수는 네이티브 Swift 객체 전용이라, `NSObject`를 상속한 Swift
클래스나 ObjC 객체를 돌려주는 API에서 참조 카운트를 깨뜨린다. Swift가 "어떤 클래스 참조"(`AnyObject`)에 쓰는
`swift_unknownObjectRetain`/`swift_unknownObjectRelease`(include/swift/Runtime/HeapObject.h, C 규약, ObjC interop 플랫폼)로
바꾼다. 네이티브 객체에는 `swift_retain`과 같고, ObjC 객체에는 `objc_retain`을 부른다. 헤더에 `APUS_C`로 선언한다.
인자로 넘길 때 잡아 두는 참조(`Owned::Object`)도 같은 쌍을 쓴다.

## API (`feature = "objc2"`, 의존성 `objc2 = "0.6"`)

| 항목 | 의미 |
|---|---|
| `impl<T: Message> From<Retained<T>> for SwiftObject` | 소유권(+1)을 그대로 넘긴다 |
| `SwiftObject::into_objc(self) -> Retained<AnyObject>` | 소유권을 그대로 넘긴다. Apple 플랫폼의 모든 Swift 클래스 인스턴스는 ObjC 객체다 |
| `SwiftObject::downcast<T: DowncastTarget>(self) -> Result<Retained<T>, SwiftObject>` | objc2의 `downcast`(`isKindOfClass`)로 확인한다 |
| `impl<T: Message> SwiftArg for &Retained<T>` | Swift 클래스 인자로 넘긴다. 호출이 끝날 때까지 참조를 잡아 둔다 |
| `impl<T: Message> SwiftRet for Retained<T>`, `Option<Retained<T>>` | 반환된 +1 참조를 받는다. `T`는 선언이 말하는 타입(다른 반환 타입처럼 확인하지 않음) |
| `impl<T: Message> SwiftReceiver for Retained<T>` | `self`로 쓴다 |

`swift!` 선언에서는 Rust 쪽에 `Retained<NSView>`를, Swift 선언 문자열에는 ObjC 클래스를 `__C.NSView(class)`로 적는다
(맹글러가 `__C`를 `So`로 맹글링한다. swiftc와 대조해 확인한다).
`&T` 대신 `&Retained<T>`를 받는 이유: `impl SwiftArg for &T where T: Message`는 기존 `&str` 등의 구현과 충돌할 수 있다(일관성 규칙).

## 검증 (적게)

테스트 probe(Swift)에 `NSObject`를 상속한 클래스와 함수 몇 개를 추가하고, `tests/swift_objc.rs` 하나로 확인한다.
- `SwiftObject` clone/drop이 `NSObject` 기반 인스턴스에서 정확하다: 마지막 drop에서 deinit이 한 번 불린다
  (수정 전에는 깨지는지 먼저 확인한다).
- `Retained<NSObject>` 인자, 반환, `self` 왕복과 `downcast` 성공/실패.
- `__C.NSObject(class)`가 들어간 선언의 맹글링이 swiftc와 같다.
