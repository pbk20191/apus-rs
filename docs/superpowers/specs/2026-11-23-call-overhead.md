# 호출 오버헤드 측정 (2026-11-23)

## 방법

- `benches/calls.rs`: `cargo bench --features test-probe --bench calls`
- 대상 함수는 `Res` 모듈(library evolution, Apple 프레임워크와 같은 방식)에 있다(`tests/support/resilient.swift`의 `bench…`).
- 각 경우를 Rust에서 `swift!`로 100만 번 부르고, 같은 일을 Swift 루프(`benchLoop…`)로 한 결과와 비교한다.
  - Swift 루프는 같은 모듈 안에 있어서 native Swift의 최선의 경우다(`BenchPoint`의 레이아웃을 안다).
- 수치는 5번 실행 중 가장 빠른 것의 호출당 시간이다. Apple M3 Pro, release 빌드.
- 프로파일: `APUS_BENCH_SPIN=add|generic|async`로 한 경우만 계속 돌리고 `sample <pid>`로 본다.

## 결과 (호출당 ns)

| 경우 | 처음 | 지금 | Swift |
|---|---|---|---|
| `Int, Int -> Int` | 29.1 | 18.8 | 0.8 |
| `String -> Int` | 49.4 | 40.4 | 0.8 |
| `Double -> 복원력 있는 struct`(`SwiftValue`) | 45.3 | 35.5 | 0.8 |
| 바인딩된 제네릭 `<T: Comparable>` | 131.2 | 81.7 | 0.8 |
| async `Int, Int -> Int` (Task 안) | 207.8 | 113.3 | 5.5 |

## 고친 것

- **호출마다 하던 조회 (`39bf9ae`):**
  - 주소로 받는 결과의 타입을 매번 mangled 이름으로 찾았다(`swift_getTypeByMangledNameInContext`).
  - 제네릭 struct receiver의 타입 이름을 매번 읽어 비교했다.
  - 둘 다 호출 지점마다 한 번만 하도록 바꿨다.
- **`#[swift_linked]` 함수 값을 static으로 (`94454e5`):**
  - 함수 값을 호출할 때마다 새로 만들어서, 그 안의 조회 캐시(`OnceLock`)가 매번 버려졌다.
  - 클래스 멤버라면 매번 `dlsym`까지 했다.
  - 호출 경로의 작은 함수들에 `#[inline]`을 붙였다. 다른 크레이트에서는 이것 없이 인라인되지 않는다.
- **async 호출 기록의 잠금 (`b61ef02`):**
  - 호출 기록에 `std::sync::Mutex`가 세 개 있었다.
  - Apple 플랫폼에서 이것은 값마다 만들고 없애는 pthread mutex라서, 호출마다 할당·초기화·해제가 일어났다.
  - 할당도 설정도 없는 `os_unfair_lock`(`src/swift/lock.rs`)으로 바꿨다.
- **즉시 실행되는 sync 호출의 빌린 `&SwiftValue` (`8ed6fb6`):**
  - 호출이 빌림보다 오래 갈 수 있다는 이유로 인자마다 복사했었다.
  - `swift!`가 바로 실행하는 sync 호출에서는 주소를 그대로 넘긴다. Swift의 borrowed(`@in_guaranteed`) 규약상 callee는 그 값을 바꾸거나 가져갈 수 없다.
  - 예외로 계속 복사하는 것:
    - callee가 consume하는 인자(initializer 인자 등). callee가 가져가는 것이 그 복사본이다.
    - 나중에 실행되는 hop 호출.
    - API를 직접 쓰는 경우.
  - consume되는 인자가 복사되는지는 테스트(`consumed_arguments_are_copies`)로 지킨다. 복사를 빼면 SIGSEGV가 난다.

## 남은 비용

**sync `Int` 호출 (약 19 ns):**
- 범용 경로 자체의 비용이다.
  - 약 480바이트의 호출 기록(`SwiftSyncCall`: 레지스터 144바이트, `ArgLowering`, `IndirectSlot`, `Option<CallError>` 두 개)을 만들고 한 번 복사한다(`memmove` 약 15%).
  - 소멸자, 검사들, 그리고 레지스터 전체를 채우는 범용 trampoline(`rs_call_sync`)이 있다.
- C FFI 호출은 1–2 ns다.

**`String` 인자:** Swift 문자열을 만드는 비용(`string_from_utf8`)과 해제가 더해진다.

**제네릭 (약 82 ns):** 대부분이 벤치마크 자체의 할당이다. 반복마다 `SwiftValue::new` 두 번과 결과 상자 하나가 `malloc`/`free`된다.

**async (약 113 ns):**
- `Arc`로 된 호출 기록, Swift async 컨텍스트 할당(`swift_task_alloc`), funclet 왕복, waker가 든다.
- Swift의 await(5.5 ns)는 같은 Task 안에서 바로 이어지므로 비교 기준이 다르다.

## 다음 후보 (아직 안 함)

1. **스칼라 인자의 sync 호출 빠른 경로:**
   - `swift!`가 인자·결과가 정수·실수뿐인 호출에 대해 `ArgLowering`이나 `Owned` 없이 레지스터만 채워 `rs_call_sync`를 부르는 코드를 생성한다.
   - ABI 가정은 지금과 같다. 같은 trampoline을 쓴다.
   - 10 ns 이하가 목표다.
2. **Rust `extern "C"`로 직접 부르기:**
   - swiftself, swifterror, swiftasync 파라미터가 없고, 스칼라만 받고, 스칼라 하나를 돌려주는 함수에 한정한다.
   - clang/LLVM의 swiftcall 구현에서는 이런 함수가 C 규약과 같다. 그래도 owner의 판단이 필요하다.
3. **`ArgLowering`의 `Vec`(`owned`, `generic_args`):** 인라인 용량을 둬서 할당을 없앤다.
