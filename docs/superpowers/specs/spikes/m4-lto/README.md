# M4 스파이크: Rust ↔ Swift cross-language 인라이닝 (LTO)

실행: `./build.sh`. 환경: rustc 1.99.1 (LLVM 23.1.1), Swift 6.4.2 / Apple clang 21, arm64.

## 결과: 양방향 인라이닝 동작

| 함수 | LTO 없음 | LTO |
|---|---|---|
| `rust_calls_swift` (Rust → Swift `swiftAdd`) | `b swift_add` | `add x0, x1, x0; ret` |
| `rust_swift_rust` (Rust → Swift `swiftMulAdd` → Rust `rust_mul`) | 호출 2번 | `madd x0, x1, x0, x2; ret` |

## 필요한 조건 (각각 실패를 확인한 것)

1. **Apple의 libLTO는 rustc bitcode를 못 읽는다** (`Invalid summary version 14. Version should be in the range [1-12]`).
   그래서 LTO는 rustc에 딸린 `rust-lld`(같은 LLVM 23)가 해야 한다. 새 LLVM이 Apple의 옛 bitcode를 읽는 방향은 된다.
2. `rust-lld`는 최신 SDK의 `.tbd`를 못 읽는다 (`arm64e.x1-macos` unknown target). 그래서 2단계로 나눈다:
   rust-lld는 LTO만 하고 object를 남기고(`-object_path_lto`), 최종 링크는 Apple ld가 한다.
3. Swift IR의 `"probe-stack"="__chkstk_darwin"`은 upstream LLVM이 코드 생성을 못 한다 (`Unsupported stack probing method`). 제거해야 한다.
4. Swift가 붙이는 `target-features` 목록이 Rust 쪽과 달라 Swift → Rust 방향 인라이닝이 막힌다. 제거하거나 맞춰야 한다.
5. Swift는 import한 C 함수 호출에 `noinline`을 붙인다 (`@_cdecl` thunk 안). Rust → Swift 안으로의 인라이닝을 막는다. 제거해야 한다.
6. **경계는 C 규약이어야 한다.** rustc에는 `extern "swift"`가 없다. Rust가 swiftcc 정의를 `extern "C"`로 선언하면
   기계어 수준에서는 같아도 LTO에서는 규약 불일치 = UB로 처리되어 `brk`가 된다 (재현함).
   Swift 쪽은 `@_cdecl`로 내보내고, Rust 함수는 C 헤더로 가져온다 (`@_silgen_name`은 swiftcc 선언이라 같은 문제).

3~5는 지금 IR 텍스트를 sed로 고치는 방식이다. 제품화하려면 bitcode를 고치는 작은 도구(LLVM 패스)나 swiftc 옵션이 필요하다.

## 확인하지 않은 것

제네릭/클래스/String이 오가는 실제 코드, async 함수, 디버그 정보, x86_64, iOS 타깃, Xcode 빌드 통합, rustc/Xcode 버전이 바뀔 때의 호환성.
