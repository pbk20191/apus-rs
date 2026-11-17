# apus-sys: ABI 헤더를 단일 기준으로 하는 sys 크레이트

작성일: 2026-11-17 · 상태: 설계 (승인 대기) · 리뷰 반영

## 1. 의도

apus는 공개 전에 Swift 런타임 ABI 층부터 정리한다. 지금은 같은 ABI를 세 곳에 따로 적는다.

| 위치 | 내용 |
|---|---|
| `csrc/swift_abi.h`, `csrc/funclets.c` | C 쪽 구조체(`SerialExecutorRef`, `AsyncContext`, …)와 런타임 함수 선언 (`SWIFTCALL`/`SWIFTASYNCCALL`) |
| `src/swift/rt.rs` | Rust 쪽 구조체, `extern "C"` 블록, `REQUIRED_SYMBOLS` 목록 |
| `src/swift/rt_sigs.txt` | `tests/abi_equivalence.rs`가 검사하는 Swift-CC 함수 시그니처 목록 |

세 곳이 어긋나도 컴파일러는 모른다. 목표는 **C 헤더 하나를 유일한 기준으로 두고**, Rust 선언, 검증, 심볼 목록을 모두
그 헤더에서 만들어 내는 것이다. 헤더와 C 코드를 담는 내부 sys 크레이트 `apus-sys`를 새로 두고 `apus`가 그것을 쓴다.

소유자 결정 (2026-11-17):
- **Apple 전용**이다. Xcode 툴체인과 OS에 들어 있는 Swift 런타임(`/usr/lib/swift`)을 전제로 한다.
- **지원 하한은 macOS 14**다. macOS 15(Swift 6.0 런타임)에만 있는 기능(TaskExecutor 등)은 실행 시 확인하는 선택 기능이다 (4.7).
- 헤더 기반 sys 크레이트를 둔다.
- Rust 선언은 **bindgen 결과를 리포에 커밋**하고 테스트로 최신인지 확인한다 (아래 3절 "선언 생성" B안).
- Swift 규약 함수 호출은 **직접 호출 + 헤더 기반 검증 + 예외만 clang thunk** (아래 3절 "호출 방식" ②안).

## 2. 목표, 비목표, 반목표

목표
- ABI 정의(구조체, 상수, 런타임 함수, 함수 포인터 모양)가 `apus-sys/include/` 헤더에만 있다.
- Rust가 Swift 런타임을 부르는 경로는 지금처럼 직접 호출이어서 인라인될 수 있다.
- 헤더에 Swift-CC 함수를 추가하면 검증이 자동으로 따라온다. 손으로 고칠 목록이 없다.
- 규약이 C와 달라지는 함수도 쓸 수 있다(clang이 만든 thunk를 거쳐서).
- `apus`의 공개 API와 동작은 바뀌지 않는다. 지금 테스트 96개가 그대로 통과한다.

비목표 (이 설계에서 하지 않음)
- 다른 의존성 정리: `dispatch2`, `objc2-core-foundation`을 feature 뒤로 옮기는 일, 외부 크레이트를 선택 의존성으로
  붙이는 일은 다음 단계에서 따로 설계한다.
- 새 Swift 기능(클래스, 프로토콜, 큰 struct 반환 등 상위 API) 추가.
- iOS/visionOS 등 다른 Apple 타깃 빌드 확인 (헤더는 그대로 쓸 수 있게 두되, 이번 검증은 macOS arm64·x86_64).
- arm64e(포인터 인증), rustc의 Swift 호출 규약, 크로스 언어 LTO feature.
- 사용자가 `swift!`로 부르는 임의의 Swift 함수 호출 경로(범용 trampoline)의 **동작**. 그 경로가 쓰는 타입(`CallRegs`, `RetRegs`,
  `AsyncFunctionPointer`)의 정의만 헤더로 옮긴다(4.3).

반목표 (이렇게 되면 실패)
- 헤더 밖에 ABI 정의가 다시 생긴다 (Rust나 테스트에 손으로 적은 구조체나 시그니처).
- `apus`를 쓰는 사람의 빌드에 bindgen이나 libclang이 필요해진다.
- 검증 없이 Swift-CC 함수를 `extern "C"`로 부르는 경로가 생긴다.

## 3. 검토한 방식

### 선언 생성 (Rust 쪽 선언을 언제 만드나)

| 안 | 내용 | 판단 |
|---|---|---|
| A | 빌드마다 bindgen | 사용자 빌드에 bindgen과 libclang이 필요해진다 |
| **B (채택)** | bindgen 결과를 커밋하고, 테스트가 다시 생성해 비교 | 사용자 빌드가 가볍고, 바인딩 변화가 diff로 보인다. objc2 계열과 같은 방식 |
| C | Rust를 기준으로 cbindgen이 헤더 생성 | `swiftasynccall` 같은 속성을 표현하기 어렵고, 헤더 기준 방향과 반대 |

### 호출 방식 (Swift 규약 런타임 함수를 Rust가 어떻게 부르나)

| 안 | 내용 | 판단 |
|---|---|---|
| ① | 전부 clang thunk | 가장 단순하지만 호출마다 함수 하나를 더 거치고, 인라인하려면 LLVM 버전이 맞는 clang으로 LTO가 필요 |
| **② (채택)** | 기계어가 같다고 증명된 함수는 `extern "C"`로 직접, 다른 함수만 thunk | 처음 결정("glue는 인라인되게 Rust가 직접 부른다")을 지키면서 기준이 하나가 된다 |
| ③ | `static inline` thunk + 항상 LTO | 사용하는 쪽까지 LTO 설정과 맞는 clang이 필요해서 공개 크레이트로는 진입 장벽이 높다 |
| ④ | 아키텍처별 어셈블리 thunk (cidre 방식) | clang이 같은 일을 하므로 이 층에서는 쓸 이유가 없다 |

rustc가 언젠가 Swift 호출 규약을 직접 지원하면 ②의 직접 호출 부분을 그쪽으로 바꾼다.

## 4. 설계

### 4.1 크레이트 구성

```
apus-rs/
  Cargo.toml            # workspace: ".", "sys", "mangle", "macros"
  sys/                  # crate apus-sys (links = "apus_abi")
    include/apus/abi.h  # 유일한 기준. 필요하면 abi/*.h로 나누고 abi.h가 모두 include
    csrc/funclets.c     # 지금 csrc/funclets.c (런타임이 되부르는 swiftasynccall 함수들)
    csrc/thunks.c       # APUS_THUNK 함수의 C 규약 thunk
    src/lib.rs          # include!("bindings.rs") + 최소한의 보조
    src/bindings.rs     # bindgen 결과 (커밋, 손으로 고치지 않음)
    build.rs            # cc로 csrc 컴파일, swiftCore/swift_Concurrency 링크, 헤더 경로를 DEP_로 공개
    tests/
      layout.rs         # bindgen이 만든 레이아웃 테스트 + _Static_assert는 헤더에
      lowering.rs       # 4.4의 기계어 비교
      fresh.rs          # 4.5의 최신 여부
  src/swift/rt.rs       # extern 블록과 구조체 정의가 빠지고, apus_sys 위의 안전한 보조 함수만 남는다
```

- `apus`는 `apus-sys = { path = "sys", version = "=x.y.z" }`로 정확히 같은 버전에 묶는다.
- `apus-sys`는 공개 크레이트지만 내부용이다. README 첫 줄에 "apus 전용, 안정성 보장 없음"을 적는다.
- `apus`의 `build.rs`에서 `cc` 컴파일과 Swift 링크 설정이 빠지고 `apus-sys`로 옮긴다. 테스트용 Swift probe 빌드는 `apus`에 남는다.
- Apple 전용: `apus-sys`의 `lib.rs`와 `build.rs`는 `target_vendor = "apple"`이 아니면 `compile_error!`로 멈춘다.
- `apus-sys`의 의존성은 `cc`(build) 하나다. `bindgen`은 dev-dependency다(4.5).

### 4.2 헤더 규칙

헤더는 swiftlang/swift의 **고정된 릴리스 태그**에서 필요한 부분만 C로 옮긴다.

- 맨 위에 `#define APUS_SWIFT_ABI_TAG "swift-6.4.2-RELEASE"`. 모든 항목 위에 원본 파일과 이름을 주석으로 단다
  (예: `// include/swift/ABI/Task.h: AsyncContext`). 원본 코드는 복사하지 않고 레이아웃만 옮긴다(Apache-2.0 원본, 출처 표시).
- 구조체마다 `_Static_assert(sizeof(…) == …)`로 원본과 같은 크기를 고정한다.
- 상수(`TaskCreateFlags` 비트, `TaskOptionRecordKind`, `MetadataKind`, `NumWords_TaskGroup` 등)는 `enum` 또는 `#define`으로 헤더에 둔다. 지금 `rt.rs`에 있는 `CREATE_*`, `METADATA_KIND_*` 상수가 여기로 온다.
- 규약 매크로:

| 매크로 | 보통 컴파일 | bindgen 실행 시 (`APUS_BINDGEN` 정의) |
|---|---|---|
| `APUS_SWIFTCALL` | `__attribute__((swiftcall))` | 빈 매크로 |
| `APUS_SWIFT_CONTEXT`, `APUS_SWIFT_ERROR`, `APUS_SWIFT_INDIRECT_RESULT` | 각 clang 속성 | 빈 매크로 |
| `APUS_SWIFTASYNCCALL`, `APUS_ASYNC_CONTEXT` | 각 clang 속성 | 해당 없음 (4.3: Rust에 보이지 않음) |
| `APUS_SYMBOL(s)` | `__asm__("_" s)` (mangled 이름, 예: `$sSqMa`) | 같음. bindgen이 `link_name`으로 옮긴다 |

- **분류 표시**: 모든 런타임 함수 선언에 다음 중 하나를 `__attribute__((annotate("…")))` 매크로로 단다. clang AST에 남고, bindgen은 무시한다.

| 표시 | 뜻 | Rust에서 |
|---|---|---|
| `APUS_C` | 원래 C 규약 함수 (`swift_retain`, `swift_dynamicCast`, …) | 직접 호출, 검증 없음 |
| `APUS_DIRECT` | Swift 규약이지만 C 규약과 기계어가 같음 | 직접 호출, 4.4가 증명 |
| `APUS_THUNK` | Swift 규약이고 C와 다름 | `apus_<이름>` thunk를 호출. 원래 선언은 Rust에 보이지 않음 |
| `APUS_ASYNC` | `swiftasynccall` (`swift_continuation_await`, `swift_task_switch`, `swift_taskGroup_waitAll`) | 보이지 않음. funclet만 부른다 |
| `APUS_INTERNAL` | C(funclet)만 부르거나 C가 구현해 런타임이 부르는 Swift-CC 함수와 typedef (`swift_task_alloc`/`dealloc`, `swift_continuation_init`, `swift_task_addCancellationHandler`, `swift_taskGroup_*`, `swift_job_run`, 취소 핸들러 `CancelFn` 등) | 보이지 않음. clang이 규약을 맞추므로 검증하지 않는다 |

- `APUS_THUNK`, `APUS_ASYNC`, `APUS_INTERNAL` 선언은 `#ifndef APUS_BINDGEN` 블록 안에 둔다. 그래서 bindgen 결과에 없다.
- Rust가 나중에 `APUS_INTERNAL` 함수를 직접 부를 필요가 생기면 `APUS_DIRECT`나 `APUS_THUNK`로 바꾸고 블록 밖으로 옮긴다.
  `swift_context` 같은 특수 레지스터로 인자를 받는 모양(`CancelFn`)은 C와 기계어가 다를 수밖에 없으므로 그때는 `APUS_THUNK`다.
- **Swift-CC `Bool` 반환**: Swift 규약은 반환값의 bit 0만 정의한다. 그래서 헤더에서는 이런 함수의 반환 타입을 `_Bool`이 아니라
  `uint8_t`로 적고(`swift_task_isCancelled`), Rust의 안전한 보조 함수가 `& 1`로 읽는다. 4.4 테스트는 Swift-CC 함수가
  `_Bool`을 반환하면 실패한다.
- **함수 포인터 모양**(value witness: `destroy`, `initializeWithCopy`, `initializeWithTake`, `getEnumTag`)은
  `typedef APUS_SWIFTCALL …`로 선언하고, 같은 `APUS_DIRECT` 표시를 typedef에 단다. 4.4가 함수와 똑같이 검증한다.
  `ValueWitnessTable` 구조체는 이 typedef들을 필드 타입으로 쓴다.
- 표준 라이브러리 심볼(`$sSqMa`, `$sSaMa`, `$sSDMa`, `$sSS10describingSSx_tclufC`, metadata와 witness table 데이터 심볼 `$sSiN`,
  `$sSSSHsWP` 등)도 같은 규칙으로 헤더에 둔다. 데이터 심볼은 `extern const uint8_t … APUS_SYMBOL("…")`.

### 4.3 Rust에 보이는 것

`bindings.rs`는 다음만 담는다.
- 구조체, typedef, 상수
- `APUS_C`, `APUS_DIRECT` 함수의 `extern "C"` 선언 (mangled 이름은 `#[link_name]`)
- `APUS_THUNK` 함수의 thunk `apus_<이름>` 선언 (평범한 C 함수)
- funclet이 Rust에 내보이는 함수(`rs_entry_fn`, `rs_context_size`, `rs_job_metadata`, `rs_run_job_fn`, `rs_string_from_utf8` 등).
  헤더의 "funclets" 절에 선언한다.
- funclet이 **Rust에서 가져다 쓰는** 콜백(`rs_poll`, `rs_park`, `rs_finish`, `rs_take_call`, …)은 Rust 쪽이 `#[no_mangle] extern "C"`로
  정의한다. 헤더에는 C가 볼 선언만 두고, bindgen은 이 절을 건너뛴다(`allowlist`/`blocklist` 설정으로).

- **호출 경로 타입**: `AsyncFunctionPointer`, `CallRegs`, `RetRegs`는 C와 Rust가 함께 쓰는 ABI이므로 레이아웃은 헤더가 정한다
  (필드 이름은 헤더 것, 예: `expected_context_size`). `apus`의 공개 타입은 바뀌지 않는다: `apus::swift::AsyncFunctionPointer`와
  `RetRegs`는 생성된 타입을 감싼 `#[repr(transparent)]` 래퍼로 두어 필드를 공개하지 않는다(설계자 판단).
  `rs_call_sync` 등 funclet 함수의 매개변수 타입도 생성된 타입을 쓴다.

`REQUIRED_SYMBOLS`(self-check가 실행 시점에 `dlsym`으로 확인하는 목록)도 같은 생성 단계에서 만들어 `bindings.rs` 옆
`symbols.rs`로 커밋한다: 헤더의 모든 런타임 함수와 데이터 심볼(표시 종류 무관, `APUS_ASYNC` 포함)의 링크 이름.

### 4.4 기계어 비교 테스트 (`sys/tests/lowering.rs`)

지금 `tests/abi_equivalence.rs`가 하는 일을 헤더에서 자동으로 한다. `rt_sigs.txt`는 없어진다.

1. `clang -Xclang -ast-dump=json -fsyntax-only`로 헤더를 읽어, `annotate` 표시가 붙은 함수 선언과 함수 포인터 typedef를 모두 뽑는다(이름, 반환 타입, 매개변수 타입, 표시).
2. `APUS_DIRECT`인 각 항목에 대해 호출 함수 하나씩을 담은 C 파일을 두 벌 만든다: 한 벌은 헤더 그대로(`swiftcall`), 한 벌은 `-DAPUS_BINDGEN`(C 규약).
3. arm64와 x86_64로 각각 `-O2` 컴파일하고 `objdump`로 호출 함수 본문을 비교한다(주소와 심볼 주석은 지금처럼 정규화).
4. 실패 조건:
   - `APUS_DIRECT`인데 기계어가 다르다 → `APUS_THUNK`로 바꾸라는 메시지
   - Swift-CC 선언(함수와 typedef)에 표시가 없다 (`APUS_DIRECT`, `APUS_THUNK`, `APUS_ASYNC`, `APUS_INTERNAL` 중 하나)
   - Swift-CC 함수가 `_Bool`을 반환한다
   - `APUS_THUNK` 함수에 대응하는 `apus_<이름>` thunk 선언이 헤더에 없다
   - `APUS_C` 함수에 Swift 속성이 붙어 있다

참고: `APUS_THUNK`가 실제로 C와 다른지(불필요한 thunk가 아닌지)는 검사하지 않는다. thunk가 남아 있어도 정확성은 clang이 보장하므로 문제되지 않는다.

### 4.5 생성과 최신 여부 (`sys/tests/fresh.rs`)

- 생성은 테스트 하나가 맡는다. `bindgen` 크레이트(dev-dependency, 버전 고정)로 `-DAPUS_BINDGEN`과 함께 헤더를 읽어
  `bindings.rs`와 `symbols.rs`를 만든다.
- 생성 파일 맨 위에 bindgen 버전과 libclang(Xcode clang) 버전을 주석으로 기록한다.
- 기본 실행: 지금 libclang 버전이 기록과 다르면 비교하지 않고 "skipped: bindings were generated with clang X, this is Y"를 출력한다
  (Xcode가 다른 사람의 `cargo test`가 헤더 변경 없이 실패하지 않도록). 버전이 같은데 결과가 다르면 실패하고 "`APUS_BLESS=1 cargo test -p apus-sys --test fresh`로 다시 생성"이라고 안내한다.
- `APUS_BLESS=1`이면 파일을 덮어쓴다.
- libclang은 Xcode 툴체인 것을 쓴다(`xcrun`으로 찾음). Apple 전용이라 다른 경로는 고려하지 않는다.
- bindgen 설정(allowlist, `layout_tests`, `default_enum_style`, 파생 trait 등)은 이 테스트 파일 한 곳에만 있다.

### 4.6 옮겨 오는 것과 없애는 것

| 지금 | 이후 |
|---|---|
| `csrc/swift_abi.h` | `sys/include/apus/abi.h`로 옮기고 4.2 규칙에 맞게 확장 |
| `csrc/funclets.c` | `sys/csrc/funclets.c`. 런타임 함수 선언은 헤더에서 include |
| `rt.rs`의 구조체, 상수, `extern` 블록, `REQUIRED_SYMBOLS` | `bindings.rs`/`symbols.rs`(생성). `rt.rs`에는 안전한 보조 함수(`is_cancelled`, `string_to_rust`, `type_name`, `any_metadata` 등)만 남는다 |
| `src/swift/rt_sigs.txt`, `tests/abi_equivalence.rs` | 삭제. `sys/tests/lowering.rs`가 대신한다 |
| `apus/build.rs`의 `cc` 컴파일과 Swift 링크 | `sys/build.rs` (`-l`, `-L`은 의존 크레이트로 전파된다) |
| `apus/build.rs`의 rpath(`-Wl,-rpath,/usr/lib/swift`) | `cargo:rustc-link-arg`는 그 크레이트의 타깃에만 적용되므로 `apus/build.rs`에 남긴다. 최종 앱도 필요하면 직접 준다(README에 적음) |

`rt.rs`의 보조 함수는 결과 타입 이름이 생성된 것으로 바뀔 수 있다(예: bindgen이 만든 `MetadataResponse`). 동작은 바뀌지 않는다.

### 4.7 지원 하한과 선택 기능

- 하한은 macOS 14다. `apus-sys/build.rs`는 C 코드를 `-mmacosx-version-min=14.0`(환경의 `MACOSX_DEPLOYMENT_TARGET`이 더 높으면 그 값)으로 컴파일한다.
  README에 하한을 적는다.
- 헤더에서 macOS 15 이상에만 있는 선언에는 `APUS_AVAILABLE(15.0)`을 단다. 보통 컴파일에서는
  `__attribute__((availability(macos, introduced=15.0), weak_import))`로, bindgen 실행 시에는 별도의 `annotate("apus:available:15.0")`로 펼쳐진다.
- **필수와 선택의 구분**
  - 표시가 없는 선언은 필수다. `symbols.rs`의 `REQUIRED_SYMBOLS`에 들어가고, self-check가 없으면 실패한다(지금과 같음).
  - `APUS_AVAILABLE`이 붙은 선언은 선택이다. `OPTIONAL_SYMBOLS`(심볼과 버전)로 따로 생성된다. self-check는 이 목록을 확인하지 않는다.
- **선택 함수를 Rust에서 부르는 방법**: Rust의 `extern` 블록은 weak 링크를 표현하지 못하므로, 선택 함수는 `bindings.rs`의 `extern` 블록에
  넣지 않는다. 생성기가 선택 함수마다 함수 포인터 타입과 처음 한 번 `dlsym`으로 찾아 두는 접근자(`Option<unsafe extern "C" fn(…)>` 반환)를
  `optional.rs`로 만든다. `APUS_THUNK`인 선택 함수는 C thunk가 weak 심볼이 NULL인지 먼저 확인하고, 이용 가능 여부를 묻는 `apus_has_<이름>()`를 함께 둔다.
- 선택 함수도 `lowering` 검증 대상이다(필수 함수와 똑같이).
- **`macos15` feature (하한을 15로 올림)** — 소유자 요청
  - `apus`의 `macos15` feature가 `apus-sys/macos15`를 켠다. 앱이 어차피 15 이상만 지원할 때 쓴다.
  - 켜면 C 코드를 `-mmacosx-version-min=15.0`으로 컴파일한다. 환경의 `MACOSX_DEPLOYMENT_TARGET`이 15보다 낮게 지정돼 있으면
    `build.rs`가 그 사실을 적은 오류로 빌드를 멈춘다.
  - 선택 선언이 필수가 된다: self-check가 `OPTIONAL_SYMBOLS`까지 확인한다.
  - 선택 함수의 `extern` 선언이 생긴다: 생성기가 `optional.rs` 옆에 `bindings_macos15.rs`(선택 함수의 보통 `extern "C"` 선언,
    `APUS_THUNK`면 thunk 선언)를 만들고, `#[cfg(feature = "macos15")]`로 포함한다. 15가 하한이므로 보통 링크로 충분하다.
  - `optional.rs`의 접근자는 feature와 상관없이 그대로 있다. feature가 켜져 있으면 `dlsym` 없이 항상 `Some(직접 선언)`을 돌려준다.
    그래서 선택 기능을 쓰는 `apus` 코드는 한 갈래로 쓰고, 15 전용 공개 API만 `#[cfg(feature = "macos15")]`로 열 수 있다.
  - `fresh`는 `bindings_macos15.rs`도 비교한다. 검증은 feature를 켠 빌드와 끈 빌드에서 기존 테스트를 한 번씩 돌린다.
- 이번 범위에는 macOS 15 전용 함수를 실제로 쓰는 코드가 없다. 위 구조와 생성기만 만들고, 헤더에 예시로 선언 하나를 넣어
  `optional.rs` 생성과 `lowering` 검증이 도는지 확인한다(어떤 함수로 할지는 빌더 판단, TaskExecutor 관련 런타임 함수 중 하나 권장).

## 5. 검증 (소유자 선호: 테스트는 적게)

- `apus` 기존 테스트 96개가 `macos15` feature를 끈 빌드와 켠 빌드에서 그대로 통과 (`--all-features`, arm64; 지금처럼 x86_64는 Rosetta로 한 번).
- `apus-sys` 테스트 세 개: `layout`, `lowering`, `fresh`.
- 새 기능이 없으므로 그 밖의 테스트는 추가하지 않는다.

## 6. 설계자가 정한 것 (리뷰 후)

- 다섯 번째 표시 `APUS_INTERNAL`: C만 부르거나 C가 구현하는 Swift-CC 선언은 Rust에 내보이지 않고 검증하지 않는다.
- 호출 경로 타입은 헤더가 레이아웃을 정하고, `apus`의 공개 타입은 `repr(transparent)` 래퍼로 유지한다.
- `fresh`는 libclang 버전이 기록과 같을 때만 비교한다.
- macOS 15 선택 함수는 `dlsym` 접근자(`optional.rs`)로 부른다. Rust `extern` 블록의 weak 링크는 쓰지 않는다.
- `macos15` feature를 켜면 접근자가 `dlsym` 없이 직접 선언을 돌려준다(접근자 API는 하나).
- `apus-sys`는 `apus`의 `swift` feature 뒤에 둔다(`dispatch`만 쓰면 빌드되지 않는다).
- 아키텍처마다 다른 선언(`GenericAsyncFn`, `CALL_GENERIC`)은 Rust에 보이지 않는 절에 둔다. `bindings.rs`는 하나다.

## 7. 빌더에게 맡기는 것

- 헤더를 한 파일로 둘지 `abi/{types,runtime,concurrency,stdlib,funclets}.h`로 나눌지.
- bindgen의 세부 옵션과 생성 파일의 포맷.
- AST JSON을 읽는 방법(`serde_json` dev-dependency로 충분할 것).
- 커밋 단위. 다만 "헤더와 생성으로 옮기기"와 "rt_sigs/abi_equivalence 삭제"는 lowering 테스트가 통과한 뒤에 한다.
