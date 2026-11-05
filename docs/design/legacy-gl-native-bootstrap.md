# Native Game: bootstrap diagnostic и видимое menu-only окно (MVP 1b/2a)

Это промежуточный startup path в **существующем Game**, не playable MVP.
Visible-window/menu gate ещё открыт: есть adapter и window-level evidence, но не
actual Game runtime с normal assets. Флаги experimental и не часть Launch API 0.1.0.

## Однокадровый диагностический маршрут

`Options::parse` → explicit `renderer=native-gl2` → validation → synchronous CPU resource load
→ `Game::new` → native owner from `pack.render` + `gui.atlas` → настоящий initial Title
→ `Game::hud` → `MenuFrame::Static` → existing native RendererFacade → один outcome → drop.

Нет winit event loop/window, `gfx::Renderer::new`, modern `PreparedTextures`, modern GPU skin
store, shader/resource packs или запуска внешних модов на этом route. Native профиль выбран
до UI/resource preparation: GUI atlas без декодирования panorama; static SDR menu. White
lightmap здесь намеренно menu-only, **не live world lighting**. World producers пока запрещены,
а не неявно урезаны до поддержанного subset. Native CLI не aliases wgpu `GraphicsApi::OpenGl`.

`--native-bootstrap-only` выбирает отдельную однокадровую диагностику вместо menu loop.
Он рисует ровно один кадр; `--frames`, если указан, допускает только `1`. World/server, control, benchmark,
headless, dev mods, session credentials, fullscreen и screenshot arguments отвергаются до
startup side effects. Native feature выключен — usage error, не modern fallback.

## Пробный запуск с собственными подготовленными ресурсами

Сначала собрать (полный client graph пока требует современный Rust/toolchain; это не Win7 artifact):

```sh
cargo build --locked -p horizon-client --no-default-features --features legacy-gl
```

Linux: нужны OSMesa/llvmpipe, normal prepared resource folder пользователя и **новые scratch folders**.
Никакие game assets не добавляются в Git и не скачиваются этим режимом.

```sh
# RESOURCE_DIR — абсолютный путь к вашим уже подготовленным ресурсам.
scratch=$(mktemp -d)
mkdir -p "$scratch/game" "$scratch/player" "$scratch/home"
HORIZON_HOME="$scratch/home" HORIZON_TELEMETRY_URL=off \
HORIZON_OSMESA_LIBRARY=libOSMesa.so.8 \
LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
MESA_GL_VERSION_OVERRIDE=2.0 MESA_GLSL_VERSION_OVERRIDE=120 \
./target/debug/horizon-client \
  --renderer native-gl2 --native-bootstrap-only --offscreen \
  --resources "$RESOURCE_DIR" --game-dir "$scratch/game" \
  --player-dir "$scratch/player" --name NativeBootstrap \
  --smoke-report "$scratch/bootstrap.json"
```

Успех этого сценария должен дать `scenario=native-game-bootstrap-only`, `backend=native-gl2`,
`menu_submissions=1`, `outcome=Offscreen`, `presented=false`, `playable=false`, `world_opened=false`.
Это завершённый offscreen draw, **не показанное окно**. Native запуск Linux без
`--offscreen` теперь использует видимый GLX owner (menu-only route ниже).

Windows: bootstrap выбирает существующий WGL owner и требует **отсутствия** `--offscreen`;
если выполнен реальный запуск, ожидается `Presented`. Это короткое окно без event loop,
не интерактивное меню и не Win7/X1650 qualification. Windows hardware здесь не проверялся.

Профиль не переписывает HDR/shader/resource-pack picks в saved options и не потребляет modern HDR recovery marker (отдельный non-destructive settings read); native bootstrap не
запускает normal onGameLoadFinished settings-save/shut_down path. Shared resource loader всё
ещё может писать штатные derived caches, имя игрока и logs — поэтому только scratch paths.

## Evidence и открытый gate

В этом patch проверяются default/full-client compile, feature-enabled Game compile и actual
Options/profile tests, native factory CPU-negative и OSMesa GPU tests. Последние используют
renderer fixtures, **не полный Game resource load**. Полный запуск указанной команды с normal
prepared resources в текущем sandbox не выполнен: такого resource folder здесь нет.

Этап 1 остаётся открытым до наблюдаемого actual Game runtime evidence; профиль для world
CPU poses/visibility ещё не подключён. Следом по [MVP-плану](legacy-gl-mvp-path.md): native visible
Linux window/input/menu adapter (начаты в MVP 2a ниже), затем responsive loading lifecycle;
не добавление новых renderer effects.


## MVP 2a: видимый native menu-only route

Текущий код также принимает **без** bootstrap flag:

```sh
# Те же new scratch game/player/home paths и ваши normal prepared resources.
# На Linux DISPLAY должен указывать на настоящий X11/XWayland/Xvfb server.
HORIZON_HOME="$scratch/home" HORIZON_TELEMETRY_URL=off \
./target/debug/horizon-client --renderer native-gl2 \
  --resources "$RESOURCE_DIR" --game-dir "$scratch/game" \
  --player-dir "$scratch/player" --name NativeMenu \
  --smoke-report "$scratch/menu.json"
```

Linux выбирает GLX + собственное X11 окно, Windows — WGL. Не создаётся winit окно/loop.
`--frames N` (N > 0) ограничивает показанные menu frames; без него — до native close/Quit/stop.
`--offscreen` по-прежнему только с `--native-bootstrap-only`.

`Game::hud` рисует тот же Title. Native adapter вызывает существующие Title hit tests,
navigation/edit/scroll methods; focus loss/overflow сбрасывают held UI input. Minimize
откладывает draw, restore не пересоздаёт context. Это menu pump, **не второй Game::idle**.
Поддержаны pointer/drag/scroll, базовые navigation keys, direct Unicode keysym text. Нет
XIM/preedit/compose, clipboard shortcuts, gameplay bindings/relative mouse и полноценного
options UI. World entry/подключение/options/external actions показывают существующий
Title error screen **до** вызова неподключённых producers. Молчаливого world skipping нет.

Существующие Title world-management действия могут менять файлы, как в modern UI;
проверять **только scratch saves**. Режим не является read-only браузером пользовательских миров.
Профиль не меняет сохранённые HDR/pack picks; явные Title preferences пользователя могут сохраняться.

В MVP 2a full CPU startup ещё происходил до native context/window. MVP 2b ниже
переносит heavy load на worker после boot window. Это всё ещё не закрытый этап 2
и не playable world. GLX window fixture
проверяется на Xvfb с llvmpipe (включая actual X11 front pixels), а не выдаётся за real Game
runtime. Normal prepared resource folder по-прежнему отсутствует в текущем sandbox.


### Проверенный срез MVP 2a

Код `6c77d27972755d4cb5d080cb9dc423cc3a896720`: собственные
[push 42012173292](https://github.com/lukintrees/horizon/actions/runs/42012173292) и
[PR 42012180338](https://github.com/lukintrees/horizon/actions/runs/42012180338),
оба 7/7 SUCCESS. Включают full-client feature compile + native_menu/startup CPU tests,
unchanged modern checks, native x86 compile/link/CPU (не Win7 runtime), real Game HUD
GL2 fixtures и отдельный `visible_glx` ignored fixture, явно запущенный на Xvfb/llvmpipe
с GL 2.0/GLSL 1.20 overrides. Это **не** normal-assets Game launch/menu evidence.
Локально: 56 standalone Rust tests, Clippy `-D warnings`, 682 Python tests (1 skipped),
fmt/inventory/public-tree/diff checks. Subsequent dead-window guard/focus refinement
должен получить собственный exact-head CI result; не наследует эти результаты.


## MVP 2b: loading window → CPU worker → тот же context в Game

Обычный visible native route теперь показывает **существующий asset-free boot HUD**
(`ui_theme::draw_boot`, wordmark/console) до dispatch heavy CPU resource load. Он не
подменяется fake Title. Существующие `load_resources` и `GameCpuResources::load` работают
на одном worker; новый renderer/backend/mesher или Game simulation не создаются. Native
profile по-прежнему vanilla/static SDR/no panorama/no modern texture preparation.

Одна generation, bounded completion channel capacity 1. Нет detached rest loader:
сейчас весь CPU bundle заканчивается до перехода к Title. Пока worker занят, render thread
помпит native resize/focus/close/minimize/restore и рисует boot screen. Это ещё не ранний
интерактивный Title параллельно загрузке world rest.

Close/stop/limited-run timeout ставят cancellation flag; checkpoints проверяются между
CPU phases. Уже идущий file read/model bake не прерывается насильно. Main thread продолжает
pump до завершения worker и делает join перед возвратом/следующей generation. Fatal GPU/
pump error тоже отменяет и join-ит worker через Drop; такой emergency cleanup может ждать
текущую CPU phase, без гарантии hard wall-clock deadline на зависшем файловом вводе.
Не допускается продолжать global/cache writes от забытой generation после выхода.

В первом срезе MVP 2b (`904da91`) boot GPU allocations удалялись после join, перед
синхронным `RendererOwner::legacy_menu_with_context`. Текущий MVP 2c ниже сохраняет drawable
boot owner до завершения GPU staging на том же Context. `Resources` остаются local owned
Arc, без Box::leak нового route. GUI font/UI-theme cache сбрасываются на publication;
loading focus переносится в Game. Ни один вариант не создаёт второе окно/device.

`--native-bootstrap-only` остаётся отдельной sync диагностикой. В normal menu report добавлены
`loading_presentations`, `loading_context_transferred` и `loading_worker_joined`.
Context-transfer/join flags независимы от числа показанных boot frames (например, при
минимизированном окне оно может быть нулевым); smoke требует и flags, и положительный счётчик. Закрытие до Game construction пишет
`scenario=native-game-loading-cancelled`, `game_constructed=false`, `worker_joined=true`;
его нельзя выдавать за menu smoke. `--frames` считает только Game menu frames, не boot frames;
loading timeout для limited run применяется до handoff, menu loop имеет собственный budget.

### Воспроизводимый normal-resource smoke (не выполнен здесь)

```sh
# Собрать feature-enabled binary, как выше. RESOURCE_DIR — только ваши ресурсы.
# Linux: DISPLAY + libGL/X11/xkbcommon; для software GL2 можно использовать xvfb-run.
python3 tools/native_game_menu_smoke.py \
  --game target/debug/horizon-client --resources "$RESOURCE_DIR" \
  --report .local/legacy-gl/native-menu.json
```

Runner сам создаёт fresh scratch home/game/player/XDG dirs, удаляет inherited HORIZON
fixture/control/session switches, сохраняет display/ICD environment и не скачивает assets.
Стартует **сам client** `--renderer native-gl2 --frames 180`, ждёт normal exit и строго
проверяет actual Game report: menu-only scenario, native backend, loading presentations,
same-context handoff, GPU checkpoints/elapsed time, Game constructed и Presented menu frames. 180 frames дают время
первоначальной onboarding fade; проверка не доказывает ручную навигацию/ввод.
Timeout/nonzero exit/missing or offscreen/bootstrap/cancelled report не принимаются за успех.
В итоговом JSON сохраняются binary hash, report и log tails, но не game assets/сохранения.
`--frames N`, `--timeout SECONDS` на runner задают budget; клиенту передаётся
`--timeout-seconds`. Exit 2 означает missing prerequisites, а не runtime PASS.

Unit mocks runner-а — только проверки isolation/report validation. OSMesa boot/HUD и GLX
same-context asset handoff fixtures — renderer/window evidence, **не normal-assets Game**.
Здесь resources folder отсутствует, полный запуск этой команды по-прежнему не выполнен.


## MVP 2c: event-aware GPU staging и atomic final publication

`LoadingScreen::owned` хранит существующий Rc-owned Context; `finish_assets` потребляет
boot owner. Candidate backend разделяет **тот же** Context/window, но имеет отдельные
tracked texture/mesh names. До его создания — checkpoint `Backend`; затем `Snapshot`,
`Layout`, `Terrain { completed: 0..N, total: N }`, `Hud`, `Glint`, `Animations`, `Ready`.
`WorldScene::new_paged_checked` вызывает callback после каждой успешной page upload,
включая последнюю перед lightmap. Старые factories продолжают использовать no-op callbacks.

На каждой границе normal route помпит события, переносит последний focus state, проверяет
stop/close/limited-run timeout и рисует тот же boot HUD. Кандидат ещё не доступен Game loop.
Boot glyph updates используют существующие textures; временные meshes ограничены 32768
GuiVertex values + u16 indices (compile-time size bound), 2 MiB и mesh object reserve.
`staging_replacement_reserving` вычитает этот запас из свободных bytes **и** object slots;
у boot owner ограничен payload budget. Перерисовка не может потратить candidate capacity.
Этот учёт — textures/meshes, **не** весь physical VRAM или driver/shader overhead.

Ошибка/отмена на любой границе удаляет partial scene/candidate/boot, не сбрасывает Game font
и не публикует renderer. После `Ready`: удалить boot allocations → восстановить полный
budget/cache capacity candidate → проверить font epoch → commit font → move полного owner
в пустой `Game.renderer`. Font epoch change во время staging отвергается. Никаких fallible
GPU uploads после font commit. Совместный budget недостаточен — явная ошибка, не скрытое
повышение лимита и не разрушение boot ради продолжения загрузки.

В normal menu report добавлены `loading_gpu_checkpoints`, `loading_gpu_presentations`,
`loading_gpu_elapsed_ms`; `loading_presentations` суммирует CPU/GPU boot presentations.
Smoke требует минимум 9 checkpoints (7 coarse phases + минимум 2 terrain boundaries),
типизированные неотрицательные GPU counters/time и согласованность frame counts. При
минимизации GPU presentations могут быть нулевыми: их не выдумывают ради PASS. GPU cancel
пишет `native-game-loading-cancelled`, `cancel_phase=gpu`, `game_constructed=true`,
`worker_joined=true`; CPU cancel — `cancel_phase=cpu`, `game_constructed=false`. Ни один
cancel report не принимается за menu evidence.

Checkpoints cooperative: отдельная shader compilation, file/decode/layout/backing operation,
texture upload или driver call не прерывается насильно. Нет доказательства hard frame-time
bound; длительность GPU phase измеряется отдельно. Это не streaming world/reload pipeline.

Регрессии: CPU scene cleanup на каждой terrain boundary; OSMesa redraw/resize/epoch commit;
отмена на всех 10 fixture checkpoints; shared budget и stale-font rejection; GLX тот же
visible window между boot draws и final menu submission. Дополнительный independent
OSMesa test через **glIsTexture/glIsBuffer** проверяет удаление настоящих partial texture
names после cancel при всё ещё живом boot context, сохраняя boot texture/buffers и transient
mesh reserve. Это renderer/window fixtures, не actual normal-assets Game acceptance.
Публичный normal-resource smoke выше всё ещё не выполнен в этом sandbox; gates 1/2 открыты.
