# Win7 x86 build audit (O01a)

Дата: 2026-11-04. Integration baseline (C01/CI01):
`74fd86ba4d7d884ff7a463c646591cbe7c6897e7`.
Цель — воспроизводимый маршрут к Windows 7 SP1 **32-bit** и Radeon X1650,
а не объявление совместимости по успешному hosted build.

**Статус:** target/distribution evidence и изолированный PE helper готовы;
реальные Rust link и Win7 runtime здесь **не выполнялись**. Hardware gate
Win7/X1650 открыт. Manifests, workflows, общая build policy и Game/GL wiring
не изменялись.

## 1. Короткий результат

| Проверка | Результат |
| --- | --- |
| `i686-win7-windows-msvc` в Rust 1.90 | Target и исходники `core`/`alloc`/`std` есть, **Tier 3** |
| Готовый std для этого target | **Нет**: target отсутствует в distribution manifest, `rustup target add` не является рабочим маршрутом |
| Обычный `i686-pc-windows-msvc` | Std доступен, но target описан как Windows 10+; годится только для ABI/compile diagnostics, не для Win7 artifact |
| Воспроизводимый `build-std` | Рекомендуется dated nightly + `rust-src` + `-Zbuild-std=std,panic_abort` |
| Native probe → полная игра | Это разные gates; независимый probe не проверяет game dependency closure |
| Фактический MSRV полной игры на baseline | Не 1.90: `horizon-client` объявляет 1.90, но безусловно зависит от `horizon-mods` с `rust-version = "1.95"` |
| PE helper | 8 unit tests проходят, включая отрицательные fixtures |
| Rust compile/link в этой среде | Не запускался: `rustc`, `cargo`, `rustup` и MSVC SDK/linker отсутствуют |
| Win7 SP1 x86 + X1650 | Стенда нет; никакого runtime qualification нет |

Первичные источники, прочитанные по точным версиям:

- Rust 1.90 [Win7 platform support](https://github.com/rust-lang/rust/blob/1.90.0/src/doc/rustc/src/platform-support/win7-windows-msvc.md):
  Tier 3, precompiled std не поставляется, предлагается `build-std` либо своя сборка Rust.
- Rust 1.90 [target spec](https://github.com/rust-lang/rust/blob/1.90.0/compiler/rustc_target/src/spec/targets/i686_win7_windows_msvc.rs):
  `pentium4`, SSE2 ABI, эмулируемый TLS, `/LARGEADDRESSAWARE` и `/SAFESEH`.
  Эмуляция TLS специально обходит проблему alignment TLS directory у Win7 x86 loader.
- Rust 1.90 [ordinary target spec](https://github.com/rust-lang/rust/blob/1.90.0/compiler/rustc_target/src/spec/targets/i686_pc_windows_msvc.rs):
  описание `32-bit MSVC (Windows 10+)`, обычный native TLS.
- Официальные manifests [1.90.0](https://static.rust-lang.org/dist/channel-rust-1.90.0.toml)
  и [nightly-2026-04-16](https://static.rust-lang.org/dist/2026-04-16/channel-rust-nightly.toml).
  В обоих `rust-std.target.i686-pc-windows-msvc` имеет `available = true`, а записи
  `rust-std.target.i686-win7-windows-msvc` нет (это не установленный, а нераспространяемый component).
- Stable 1.90.0 — rustc commit `1159e78c4747b02ef996e55082b704c09b970588`.
  Выбранный nightly — rustc **1.97.0-nightly**, commit
  `e8e4541ff19649d95afab52fdde2c2eaa6829965`; его dated manifest и `rust-src` существуют.

Тот же Tier-3/no-precompiled-std статус сохраняется в исходниках 1.95 и 1.99;
простая смена stable на более новый stable не устраняет необходимость `build-std`.

## 2. Варианты сборки и выбор

### 2.1. Только диагностика ordinary target

На машине с rustup следующие команды различают наличие target в rustc и component в dist:

```powershell
rustup toolchain install 1.90.0 --profile minimal
rustup run 1.90.0 rustc -vV
rustup run 1.90.0 rustc --print target-list | Select-String '^i686-win7-windows-msvc$'
rustup target list --toolchain 1.90.0 | Select-String 'i686-(pc|win7)-windows-msvc'
rustup target add --toolchain 1.90.0 i686-pc-windows-msvc
```

Последняя команда относится только к ordinary ABI probe. Попытка заменить её на
`rustup target add ... i686-win7-windows-msvc` противоречит distribution manifest
и не должна обходиться копированием std от `i686-pc-windows-msvc`: у target различаются
OS support и TLS strategy.

### 2.2. Собственный stable sysroot

Официально возможен второй путь: собрать Rust 1.90.0 из точного tag с
`i686-win7-windows-msvc` в `bootstrap.toml`, затем использовать полученный stage-1
sysroot. Это тяжёлый отдельный toolchain release process, а не `rustup target add`.
Он не выбран для первого OS probe и здесь не выполнялся. Для воспроизводимого
release такой вариант требует сохранить source commit, bootstrap config, host tools,
SDK, sysroot archive и hashes; смешивать самосборный std с другим rustc нельзя.

### 2.3. Рекомендуемый первый путь: dated nightly + build-std

Использовать `nightly-2026-04-16`, а не плавающий `nightly`. Версия новее заявленного
MSRV 1.90 **и** фактических 1.95 текущего game graph. Она не доказывает, что полная
игра собирается именно компилятором 1.90; такое заявление на этом baseline неверно.

Нужен Windows build host с x86 MSVC linker, Windows SDK и resource tools. Консервативная
отправная комбинация для проверки — VS 2019 16.11 x86 tools + SDK 10.0.19041.0;
она **не проверена** с этим nightly и game graph. Не переключаться молча на newest SDK:
записать `link`, `rc`, `mt`, SDK и CRT versions, затем аудировать полученный PE.
Build scripts выполняются на host, поэтому нужны и host Rust tools, и x86 target C/SDK tools.

PowerShell из корня репозитория:

```powershell
$Toolchain = "nightly-2026-04-16"
rustup toolchain install $Toolchain --profile minimal --component rust-src
rustup run $Toolchain rustc -vV
rustup run $Toolchain cargo -V
rustup component list --toolchain $Toolchain --installed
rustup run $Toolchain rustc --print target-list | Select-String '^i686-win7-windows-msvc$'
where.exe link
where.exe rc
where.exe mt
link /?

# Не наследовать target-cpu=native или чужие target features.
$env:RUSTFLAGS = "-Ctarget-cpu=pentium4 -Ctarget-feature=+crt-static"
```

`+crt-static` выбран для уменьшения CRT deployment closure и должен применяться
одинаково к std и приложению. Это не запрет на Win8/10 APIs и не доказательство
отсутствия CRT/OS dependencies. Перед native C dependencies дополнительно задать
`CFLAGS_i686_win7_windows_msvc = "/arch:SSE2 /MT"`, проверить фактические build logs:
не каждый native build script обязан honour этот environment variant.

## 3. Gate 1: существующий native probe

Probe независим от root game workspace; его crates объявляют MSRV 1.85.
Сначала **собрать и слинковать**, а не ограничиться `cargo check`:

```powershell
cargo +$Toolchain -Zbuild-std=std,panic_abort build `
  --locked --release `
  --manifest-path tools/legacy-gl-probe/Cargo.toml `
  --target i686-win7-windows-msvc `
  --target-dir target/legacy-win7/probe

$Probe = "target/legacy-win7/probe/i686-win7-windows-msvc/release/horizon-legacy-gl-probe.exe"
python tools/legacy_win7_audit.py $Probe --expected-arch i686 `
  --json target/legacy-win7/probe-pe.json
if ($LASTEXITCODE -ne 0) { throw "probe PE audit failed" }
```

Audit каждой поставляемой DLL выполняется тем же ожиданием `i686`; одного EXE недостаточно.
После переноса на чистый Win7 SP1 x86 стенд:

```powershell
.\horizon-legacy-gl-probe.exe --context wgl --shader-version 110
```

Сохранить JSON, stderr, binary SHA-256, версии OS/CPU/driver. Нужны реальные старт,
heap/threads/files/TLS, WGL window, shader compile, readback и `SwapBuffers`.
Существующий probe выделяет память через `Vec` и проверяет GL pipeline, но не имеет
отдельных thread/file/TLS checkpoints: WGL success не закрывает эти OS cases. Они требуют
отдельного минимального OS probe/сценария O01/O03; этот PR не выдает GL probe за него.
`--require-llvmpipe` на X1650 не использовать. Microsoft GDI Generic — отсутствие
аппаратного ICD, а не прохождение GL2 gate.

**В этой среде эти build/runtime команды не выполнялись.** Existing
`i686-pc-windows-msvc` CI artifact и Windows-latest execution не закрывают этот gate.

## 4. Gate 2: текущая полная игра — отдельно

На baseline `crates/horizon-client/Cargo.toml` и `horizon-client-lib` требуют 1.90,
но оба имеют обязательную dependency `horizon-mods`; её manifest требует 1.95,
и `Cargo.lock` подтверждает связь `horizon-client → horizon-mods`. Поэтому toolchain
1.90 нельзя рекламировать как рабочий для полного текущего графа. Нужно решение
владельца manifests/D01: сохранить новый фактический MSRV либо отдельно переработать
legacy dependency graph. Этот PR не меняет manifests и не обходит dependency вторым API.

Диагностическая link-команда для **текущего, ещё не legacy-only** графа:

```powershell
cargo +$Toolchain -Zbuild-std=std,panic_abort build `
  --locked --release -p horizon-client --bin horizon-client `
  --no-default-features `
  --target i686-win7-windows-msvc `
  --target-dir target/legacy-win7/game-current-not-qualified
```

Даже успех здесь будет только compile/link evidence:

- `--no-default-features` отключает audio/controllers, но не удаляет обязательные
  wgpu/winit/DX12/Vulkan/GLES и modern dependency paths;
- это не D01 legacy-only artifact, не Game startup и не B3;
- `horizon-mods` остаётся в графе; build scripts и transitive native dependencies
  требуют отдельного аудита;
- PE audit EXE и всего DLL closure обязателен, но тоже не делает сборку совместимой.

После D01/C01/R01 wiring тот же target/toolchain маршрут повторяется для настоящего
legacy-only Game artifact. Затем настоящий B2 сценарий и перенос на Win7/X1650 —
отдельные gates T02/O02, а не следствие успешного link.

## 5. PE/import helper

`tools/legacy_win7_audit.py` использует только Python stdlib, не импортирует repo build
policy и не запускает исследуемый файл. Он сообщает:

- COFF machine, PE32/PE32+, consistency, DLL flag, Large Address Aware;
- image base и declared OS/subsystem versions;
- ordinary и delay imports, DLL, имена/hint и ordinal imports;
- оба delay descriptor modes (`grAttrs`: RVA или absolute VA);
- CRT-looking dependencies (`msvcrt`, MSVC runtime, UCRT/API-set) по имени DLL;
- SHA-256 и размер каждого файла, machine-readable JSON.

Exit codes: `0` — все файлы разобраны и architecture соответствует; `1` — architecture
mismatch; `2` — malformed/unreadable input. CRT dependency сама по себе не является
parser failure: её нужно расследовать и либо статически устранить, либо поставлять
поддерживаемый Win7 x86 runtime. Пустой список CRT imports не доказывает статический
CRT или отсутствие runtime prerequisites. Классификация CRT — name heuristic.

Для нескольких файлов:

```powershell
$Images = @($Probe) + (Get-ChildItem .\payload -Filter *.dll).FullName
python tools/legacy_win7_audit.py $Images --expected-arch i686 --json payload-pe.json
```

Тесты и наблюдаемый результат в этой среде:

```text
python3 -m unittest discover -s tests -p 'test_legacy_win7_audit.py' -v
Ran 8 tests
OK
python3 tools/legacy_win7_audit.py --help   # exit 0
```

Synthetic fixtures покрывают i686/PE32 и x86_64/PE32+, 4/8-byte thunks,
ordinary/delay/named/ordinal imports, RVA/VA delay tables, `FirstThunk` fallback,
CRT, wrong architecture, malformed PE/RVA/descriptors/thunks и валидный JSON stdout.
Бинарные fixtures создаются тестами независимо от parser и не требуют Windows/objdump.

Дополнительный interoperability smoke выполнен на реальном `cargo.exe` из pinned
npm mirror package `@rustbin/cargo-1.88.0-i686-pc-windows-msvc@0.89.0`
(registry SHA-512 integrity проверен npm). Parser сообщил PE32/i686 и 15 ordinary DLL;
независимый `objdump -p` перечислил те же 15 DLL. Delay и CRT-looking imports в этом
файле отсутствуют. Воспроизведение на Linux:

```bash
npm install --prefix /tmp/legacy-win7-pe-smoke --ignore-scripts --no-audit --no-fund \
  @rustbin/cargo-1.88.0-i686-pc-windows-msvc@0.89.0
CargoPe=/tmp/legacy-win7-pe-smoke/node_modules/@rustbin/cargo-1.88.0-i686-pc-windows-msvc/cargo/bin/cargo.exe
python3 tools/legacy_win7_audit.py "$CargoPe" --expected-arch i686 \
  --json /tmp/legacy-win7-pe-smoke/cargo-pe.json
objdump -p "$CargoPe" | grep 'DLL Name:'
```

Это только parser smoke на ordinary Windows binary, не запуск и не Win7 qualification;
third-party binary не добавлен в репозиторий или test fixtures.

## 6. Риски, которые нельзя снять import table

### Dynamic API и DLL loading

- `LoadLibrary*`/`GetProcAddress`, WGL extensions, COM, driver и API-set resolution
  не исчерпываются import descriptors. Нужны source inventory и runtime loader/API trace
  на Win7; delay-loaded DLL тоже входит в closure.
- Отсутствие Win8/10 имени в imports не доказывает, что оно не разрешается динамически.
  Forwarded export и присутствие DLL не гарантируют наличие конкретной функции.
- Сеть/TLS/certificates, audio/controllers/narrator включаются и проверяются по одному
  в O03. Нельзя «чинить» TLS отключением certificate verification.
- `windows-segment-heap` не включать: его manifest относится к Windows 10 2004+.
  Он не входит в default features, но PE helper не проверяет manifest.

### Allocator и 32-bit address space

- Default client использует mimalloc и вызывает `horizon_threads::configure_allocator()`
  в `entry.rs`. Нужен отдельный x86 C/ISA/import/runtime audit mimalloc и его patched source.
- `system-allocator` меняет только `#[global_allocator]`; mimalloc остаётся dependency,
  а `configure_allocator()` вызывается безусловно. Feature не исключает allocator risk.
- `/LARGEADDRESSAWARE` отображается helper, но не измеряет память. На Win7 x86 нужно
  измерить peak virtual address space, fragmentation, commit и upload/resource peaks;
  RAM или VRAM сами по себе не отвечают на вопрос о 32-bit address space.

### CPU ISA

- Target baseline `pentium4` означает обязательный SSE2; X1650 ничего не говорит о CPU.
  Если реальный CPU без SSE2, этот target не подходит и нужен отдельный custom-target проект.
- Не использовать `target-cpu=native`. Rust flags не контролируют все C/C++ objects,
  precompiled libraries и runtime dispatch.
- Existing `tools/check_cpu_baseline.py` проверяет узкий x86_64 POPCNT/LZCNT policy,
  а не полный i686 ISA audit. Нужны дизассемблирование всех PE/DLL, анализ SSE3/SSSE3/
  SSE4/AVX/POPCNT и guards, затем запуск на реальном старом CPU. LLVM `llvm-objdump`
  полезен как evidence, но отсутствие выбранных mnemonic тоже не доказывает корректность.

### CRT и SDK

- `msvcrt.dll` существует в Win7, но нужная функция/версия этим не гарантируется;
  UCRT и VCRedist нельзя считать автоматически установленными. Нужен x86 runtime,
  поддерживающий Win7, либо подтверждённый static CRT.
- Static CRT не убирает `kernel32`/другие OS imports. Новые SDK/import libraries могут
  ввести более новые dependencies; declared OS/subsystem version — только поле заголовка.
- Existing `tools/check_imports.py` содержит Windows 10/11 system allowlist. Его success
  нельзя переиспользовать как Win7 allowlist или как доказательство CRT deployment.

## 7. Что статический audit не доказывает

PE helper не загружает image и не доказывает: наличие DLL/functions и корректный search
order; отсутствие dynamically resolved APIs; ISA безопасность; TLS layout/emulation;
SAFESEH/CFG/load-config correctness; manifest и driver compatibility; CRT initialization;
DLL signatures; ресурсы и их подготовку; window/input/GL behaviour; сохранение мира;
performance или отсутствие fragmentation/leaks. Malformed-input rejection лишь означает,
что parser не выдаёт повреждённый файл за проверенный.

Wine, Windows-latest, ordinary x86 CI, Linux llvmpipe и synthetic fixtures полезны для
разработки, но не заменяют чистый Win7 SP1 x86 + vendor ATI ICD на X1650.

## 8. Открытые проверки и порядок закрытия

1. На build host выполить §2.3, сохранить `rustc -vV`, component/SDK/CRT versions,
   dated manifest и hashes; подтвердить link native probe через §3.
2. Проверить EXE и рекурсивный ordinary/delay DLL closure через helper; разрешить все CRT
   и не-system dependencies, затем повторить audit на точных shipped files.
3. Выполнить disassembly/ISA и dynamic API inventory; проверить allocator, TLS, threads,
   files, timers и 32-bit memory peaks. Это ещё не GPU test.
4. На чистом Win7 SP1 x86 запустить probe без переноса build sysroot; зафиксировать CRT
   prerequisites, startup и loader failures. Wine результат маркировать отдельно.
5. На X1650 подтвердить vendor ICD, GL/GLSL и limits, WGL/readback/swap; только затем
   закрывать первый OS/GPU probe gate.
6. Интегратору/D01 решить фактический MSRV 1.95 и удалить modern dependency closure;
   собрать настоящий legacy-only Game, не путая его с §4.
7. По одному вернуть network/TLS, audio, controllers, narrator; после каждого повторить
   imports/ISA/runtime. Выполнить B2 с input/save/reopen и длительный memory/performance test.
8. Повторить тот же сценарий на Win7/X1650 и оформить O02/B3 evidence. До этого статус —
   **unverified, hardware gate open**, а не accepted Win7 release.
