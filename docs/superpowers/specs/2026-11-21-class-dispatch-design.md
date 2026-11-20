# 클래스 멤버는 Swift처럼 override로 dispatch한다

## 왜

- library evolution 없이 컴파일한 모듈(`Probe` 같은 것)에는 non-final 멤버의 dispatch thunk(`…Tj`)가 없다.
- 그래서 apus는 멤버 자신의 심볼(부모의 구현)을 직접 불렀다. 하위 클래스 인스턴스에서 override가 무시되었다.
- 소유자가 2026-11-21에 고치기로 정했다. 같은 날 특수화 generic의 witness table을 호출 지점에 캐시하는 것도 정했다.
  Swift 이미지는 내릴 수 없으므로 한 번 찾은 것은 계속 유효하다(소유자 의견).

## 제약 (ABI)

- `swift_lookUpClassMethod(metadata, method, description)` (include/swift/Runtime/Metadata.h): 클래스 vtable에서 메서드
  descriptor(`…Tq`)의 구현을 찾는다. 런타임이 export하고 SDK `.tbd`에도 있다.
  - `description`은 메서드를 선언한 클래스의 descriptor(`…Mn`)이다.
  - `metadata`는 그 클래스나 하위 클래스여야 한다. 런타임은 assert만 한다. 그래서 apus가 먼저 확인한다.
- 하위 클래스 확인에 쓰는 것:
  - `swift_getTypeContextDescriptor`(Metadata.h)
  - 클래스 metadata의 superclass 필드(include/swift/ABI/Metadata.h `TargetAnyClassMetadata`)
  - metadata kind(include/swift/ABI/MetadataKind.def: 클래스는 isa 포인터로 시작하고 0x7FF보다 크다)
- 실험(2026-11-21, non-LE dylib)으로 확인한 것:
  - `Tq`는 non-final 멤버에만 export된다. final 멤버에는 `Tq`도 `Tj`도 없다.
  - `swift_lookUpClassMethod`는 override 구현과 상속된 구현을 돌려준다.
  - async 메서드에 대해서는 override의 async function pointer(`…Tu`)를 돌려준다.

## 설계

- 클래스 자신의 멤버(extension 멤버와 init 제외)의 호출 대상을 다음 순서로 찾는다.
  1. `…Tj`가 있으면 그것을 부른다.
  2. 없고 `…Tq`와 클래스 `…Mn`이 있으면 vtable에서 찾는다. `self`를 바인딩한 뒤 호출할 때 찾는다.
     - 인스턴스 멤버는 객체의 타입으로 찾는다. static(`class`) 멤버는 `self`인 metadata로 찾는다.
     - 하위 클래스가 아니면 `InvalidArgument`이다.
  3. 둘 다 없으면 자기 심볼을 부른다(final 멤버).
- `#[swift_linked]`도 같은 순서를 따른다. `Tj`와 `Tq`는 `dlsym`으로 찾는다. export하지 않은 정적 archive에서는 3으로 떨어진다(한계).
- 심볼 찾기는 호출 지점마다 한 번만 한다.
- 특수화 generic의 캐시:
  - 묶은 타입의 metadata, witness table, superclass/associated type 검사 결과를 호출 지점 `static`에 한 번 저장한다.
  - 실패는 저장하지 않는다. 나중에 로드된 이미지가 타입을 가져오면 그때 찾는다.
  - 인자가 가진 타입 검사는 호출마다 한다.

## 테스트

- `tests/swift_class_dispatch.rs`: `Probe.Speaker`/`Loud`. 다음을 부른다.
  - `#[swift]`, `#[swift_linked]`, async, `class func`의 override
  - final 멤버
  - 다른 클래스의 receiver(거부)
- RED: override 대신 부모의 구현이 불렸다.
