# apus 내부 경계 심볼: 충돌 방지와 크로스 언어 LTO

작성일: 2026-11-17 · 상태: 승인 (소유자 "continue")

## 1. 의도

apus의 C 코드(funclet, thunk)와 Rust 코드는 서로 이름으로 부른다(`rs_poll`, `rs_entry_fn`, `apus_swift_task_isCancelled` …).
지금 이 이름들은 평범한 전역 심볼이라 세 가지 문제가 있다.

- **다른 라이브러리와 이름 충돌**: `rs_poll`을 정의한 다른 라이브러리와 정적 링크하면 `duplicate symbol`로 실패한다.
  `-fvisibility=hidden`은 동적 export에서만 빼므로 이 충돌은 막지 못한다(실험으로 확인).
- **외부 노출**: strip하지 않은 실행 파일과 dylib의 export 목록에 남는다.
- **LTO**: 언어 경계를 넘는 인라인 여지를 반드시 살려야 한다(소유자 요구). 함수 포인터 표나 `ld -r` 사전 링크처럼
  인라인을 막는 방식은 쓰지 않는다.

소유자 결정 (2026-11-17)
- 우리 내부 경계(서브모듈끼리, C↔Rust)는 LTO와 크로스 모듈 인라인 여지를 반드시 살린다.
- 공개 인터페이스와 외부 모듈 쪽의 LTO·인라인은 사용자 선택으로 둔다.
- 다른 라이브러리와의 심볼 충돌 가능성을 막는다.

## 2. 실험으로 확인한 사실 (2026-11-17, macOS 27.3, rustc 1.99.1 / LLVM 23.1.1)

| 사실 | 근거 |
|---|---|
| 의존 크레이트의 `cargo::rustc-link-arg`는 다른 크레이트의 최종 링크에 전달되지 않는다 | `--print link-args`로 확인: apus-sys 자신의 테스트 링크엔 있음, 다른 크레이트엔 없음 |
| `links` 메타데이터 연쇄 + 최종 패키지 `build.rs`로는 전달할 수 있다 | 실험 성공. 단 최종 패키지에 코드가 필요 |
| `links = "apus_abi"` 때문에 apus 두 버전은 한 그래프에 들어올 수 없다 | Cargo 규칙 |
| hidden 심볼도 정적 링크에서는 같은 이름과 충돌한다 | C 세 파일 실험: `duplicate symbol` |
| rustc fat LTO만으로는 `rs_*`가 전역·export로 남는다 | export 15 |
| **xLTO(C를 LLVM 23 bitcode로, Rust는 `-Clinker-plugin-lto`)에서는 양방향 인라인되고 export 0** | Rust→C 함수는 바이너리에서 사라지고, C funclet `step` 안의 `rs_poll`·`rs_park`·`rs_take_call` 등이 인라인됨(`rs_finish`만 호출로 남음), 실행 정상 |
| C→Rust 인라인은 C도 rustc와 같은 CPU(`-mcpu=apple-m1`)로 컴파일해야 된다 | 없으면 LLVM이 target feature 불일치로 거부 |
| macOS에서 rustc는 ELF용 `-plugin-opt=…`를 링커에 넘긴다 | Apple ld와 ld64.lld 모두 거부 → 걸러 내는 래퍼 필요 |
| ld64.lld(rust-lld)는 macOS 27 SDK의 TBD(`arm64e.x1`)를 읽지 못한다 | Apple `ld` + `-lto_library <LLVM 23>/lib/libLTO.dylib`로 링크해야 한다 |

## 3. 목표, 비목표, 반목표

목표
- 내부 경계 호출은 직접 호출로 남아, xLTO에서 양방향 인라인된다.
- LTO 없는 빌드에서도 우리 경계 심볼이 다른 라이브러리의 이름과 겹치지 않는다.
- C가 정의하는 경계 심볼은 어떤 빌드에서도 동적 export되지 않는다.
- `xlto` feature로 크로스 언어 LTO를 켤 수 있고, 툴체인이 맞지 않으면 이유를 알리고 빌드를 멈춘다.

비목표
- 공개 API(`apus::…`)와 외부 크레이트 사이의 LTO 설정(사용자 선택).
- Rust가 정의하는 콜백의 동적 export 제거(stable Rust로 불가). 이름 충돌은 접두사로 막고, xLTO에서는 사라진다.
- ld64.lld 지원(SDK TBD 문제), x86_64 xLTO 검증(설계는 대응하되 이번 검증은 arm64).

반목표
- 함수 포인터 표, `ld -r` 사전 링크 등 인라인을 막는 구조.
- 사용자에게 링커 설정 없이는 빌드가 안 되는 기본 모드.

## 4. 설계

### 4.1 경계 심볼 이름공간

- 우리가 정의하는 모든 경계 심볼(C 정의: funclet export와 thunk, Rust 정의: funclet이 부르는 콜백)의 **링크 이름**은
  `apus_<tag>_<C 이름>`이다. C 식별자는 그대로 두고, 헤더의 asm label로 링크 이름만 바꾼다
  (`APUS_BOUNDARY(rs_poll)` → `__asm__("_apus_<tag>_rs_poll")`).
- `<tag>` = `sys` 버전(`.`→`_`) + `_` + 그 버전 문자열의 FNV-1a 하위 32비트 16진수 8자리. 예: `0_1_0_1a2b3c4d`.
  헤더의 `#define APUS_TAG "…"`가 유일한 정의이고, `fresh` 테스트가 `CARGO_PKG_VERSION`에서 계산한 값과 다르면 실패한다
  (버전을 올리면 헤더도 고치게 됨).
- bindgen은 asm label을 `#[link_name]`으로 옮기므로 Rust가 부르는 C 함수는 자동으로 맞는다.
- Rust 콜백은 `#[no_mangle]` 대신 `#[export_name = apus_sys::boundary_name!(rs_poll)]`. 매크로는 `fresh`가 헤더에서
  생성한다(`src/boundary.rs`). 매크로가 attribute 값에 쓰일 수 없으면 생성기가 상수 대신 리터럴을 직접 쓴 매크로를 만든다.
- `REQUIRED_SYMBOLS` 제외 규칙(C 이름 접두사 `rs_`/`apus_`)은 그대로다.

### 4.2 C 가시성

- `apus-sys/build.rs`는 C를 `-fvisibility=hidden`으로 컴파일한다. `-fvisibility`는 정의에만 적용되므로 Swift 런타임 선언은 영향이 없다.

### 4.3 `xlto` feature

- `apus-sys/xlto`, `apus`의 `xlto = ["swift", "apus-sys/xlto"]`.
- `build.rs` (feature가 켜졌을 때)
  - C 컴파일러의 LLVM 주 버전(`<cc> --version`의 `clang version N`)과 rustc의 LLVM 주 버전(`$RUSTC -vV`의 `LLVM version: N`)이
    다르면 두 값과 설정 방법(`CC_<target>`)을 적어 멈춘다. Apple clang도 버전이 다르면 같은 오류.
  - `-flto=full`, 그리고 rustc의 target CPU: `CARGO_ENCODED_RUSTFLAGS`의 `-Ctarget-cpu`/`target-cpu=`가 있으면 그 값,
    없으면 aarch64는 `apple-m1`, x86_64는 rustc의 기본값(`rustc --print target-cpus`의 default로 확인해 상수로 둠).
    arm64는 `-mcpu=`, x86_64는 `-march=`.
  - archiver는 같은 디렉터리의 `llvm-ar`.
- 링커 쪽은 사용자 설정이다. 리포에 `scripts/xlto-link`(rustc의 `-plugin-opt` 인자를 걸러 내고, LLVM 23 clang이
  Apple `ld`와 `-lto_library`로 링크)를 두고, README에 `.cargo/config.toml` 예시를 적는다(`linker`, `-Clinker-plugin-lto`,
  `CC_*`, `profile.release.lto = "fat"`). LLVM 위치는 `APUS_LLVM`(기본 `/opt/homebrew/opt/llvm`).

### 4.4 검증 (소유자 선호: 적게)

- smoke 테스트 하나(`tests/symbols.rs`, swift feature): 자기 실행 파일의 `dyld_info -exports`를 읽어
  (1) C가 정의한 경계 심볼이 하나도 export되지 않고, (2) export된 `rs_*`/`apus_*` 이름은 모두 `apus_<tag>_` 접두사를 가진다.
- `scripts/check-xlto.sh`: 위 설정으로 `swift_basic`을 release 빌드해 실행하고, export에 경계 심볼이 없으며
  `step` funclet이 `rs_poll`을 호출하지 않는지(인라인됨) 디스어셈블로 확인한다. LLVM이 없으면 건너뛴다. `cargo test`에는 넣지 않는다.
- 기존 테스트 97개, `lowering`, `fresh`는 그대로 통과.

## 5. 빌더에게 맡기는 것

- `APUS_BOUNDARY` 매크로 모양, 생성 매크로 파일의 형태.
- `scripts/xlto-link`의 세부(인자 필터, 오류 메시지).
- x86_64 기본 CPU 상수 값(실측해서 둠).
