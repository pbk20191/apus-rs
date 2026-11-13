# M3 스파이크: Swift class, struct, protocol을 ABI로 다루기

실행: `./build.sh` (모듈 `Spike`를 일반 모드와 `-enable-library-evolution` 모드로 빌드해 둘 다 실험한다).
환경: macOS 27.3 arm64, Swift 6.4.2. x86_64는 확인하지 않았다.

## 결과

| # | 실험 | 일반 모듈 | resilient 모듈 |
|---|---|---|---|
| C1 | `Animal(name:)` allocating init: 인자 + context 레지스터에 metatype | 동작 | 동작 |
| C2 | 가상 호출: `swift_lookUpClassMethod(isa, 메서드 descriptor Tq, 클래스 descriptor Mn)` | 동작 (`"Woof"`) | 동작 |
| C2 | 메서드 심볼 직접 호출 | override를 무시함 (`"..."`) | 심볼 없음 |
| C2 | dispatch thunk `Tj` | 심볼 없음 | 동작 |
| C3 | stored property: setter/getter 호출 | 심볼 직접 | `Tj` thunk |
| C3 | stored property: 인스턴스 메모리 직접 읽기 (reflection 오프셋) | 동작, `Wvd` 전역과 일치 | 동작 (`Wvd` 없음) |
| C4 | `swift_reflectionMirror_recursive*`로 필드 이름/타입/오프셋 (상위 클래스 포함) | 동작 | 동작 |
| C5 | `swift_getObjectType`, `swift_dynamicCastClass` | 동작 | 동작 |
| S1/S2 | 작은 struct를 레지스터로 만들고 넘기기 (`Point` d0,d1 / `Tag` x0..x2 / `Mixed` x0,d0,x1) | 동작 | 해당 없음: 크래시 (전부 주소로 전달) |
| S2 | 메서드의 self: 인자 뒤에 오는 마지막 인자들 (context 레지스터 아님) | 확인 | - |
| S3 | mutating 메서드: self 주소를 context 레지스터에 | 동작 | 동작 |
| S5/S6 | 주소로 전달되는 struct (큰 struct, resilient struct) + reflection 오프셋으로 필드 읽기 | 동작 | 동작 |
| P1 | `any Greeter` 열기: 5 word (버퍼 3 + metadata + witness table), witness table 항목 호출 | 동작 | 동작 |
| P2 | 요구사항 dispatch thunk, `(Tq − TL) / 8` = witness 인덱스 | 심볼 없음 | 동작 |
| P3 | existential을 주소로 Swift 함수에 전달 | 동작 | 동작 |
| P5 | **Rust로 Swift protocol 구현**: 손으로 만든 witness table + `UnsafeMutableRawPointer`를 Self로 | generic, existential 모두 동작 | 동작 |

## 알게 된 것

- 일반 모듈과 resilient 모듈은 내보내는 심볼이 정반대다. 일반: 구현 심볼, `Wvd`, 클래스 메서드 `Tq`. resilient: `Tj` thunk, `Tq`.
  둘 다에서 되는 길은 클래스 메서드 = `swift_lookUpClassMethod`, 필드 = reflection 오프셋이다.
- 일반 모듈의 protocol 요구사항은 `Tq`도 `Tj`도 없어서 witness table 인덱스를 선언 순서로 세어야 한다
  (0번은 conformance descriptor, 그다음 상속 protocol과 associated type, 그다음 요구사항. getter/setter/modify는 각각 한 칸).
- witness 메서드 규약: 인자, Self metadata, witness table 순서로 넘기고 self는 주소로 context 레지스터에 넣는다.
- struct의 non-mutating 메서드에서 self는 인자 뒤에 값으로 붙는다. 한 word짜리 struct도 x0다.
- `Mixed` 같은 정수+실수 혼합 반환은 지금 `rs_call_sync`가 한 종류만 읽어서 두 번 호출해야 했다 (기존에 알던 빈틈).
- 손으로 만든 witness table의 0번(conformance descriptor)을 NULL로 뒀는데 정적 호출에는 문제가 없었다.
  `as? any Greeter` 같은 동적 캐스트에는 보이지 않는다 (`swift_registerProtocolConformances`가 필요, 미실험).

## 실험하지 않은 것

async 요구사항(`greetAsync`), 버퍼에 안 들어가는 existential(박스), associated type이 있는 protocol의 Rust 구현,
클래스를 Rust에서 상속하기, enum, x86_64.
