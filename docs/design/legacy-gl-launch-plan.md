# От проверенного GL2 renderer к запускаемой игре: план и карта параллельных PR

Дата: **2026-11-04**. Цель: **Radeon X1650, Windows 7 SP1 32-bit, desktop OpenGL 2.0/2.1**.
Срез кода: `d092519` (проверенный код `d760a9a`, затем только документация).
Это актуальный план следующих работ; предыдущий [дизайн и журнал этапов](opengl2-win7-x86.md) сохраняет историю.

**Правила интеграции и владения:** [legacy-gl-integration.md](legacy-gl-integration.md).
Пользователь назначил главным интегратором координирующего Arena.ai Agent от имени `lukintrees`.
Общий facade/contracts и центральные manifests/lockfiles пишет **только интегратор**;
другие исполнители направляют запросы и работают в согласованных modules.

**Начальный кодовый baseline C01/CI01:** [C01 v1](legacy-gl-c01.md). Submission/scheduling wiring,
CI01a/b/c и R01a CPU Frame/related DTO extraction уже добавлены; owning facade, remaining
visibility/resource split и настоящий legacy startup ещё не завершены. CI01d/branch
protection/main-side trusted gate bootstrap не выдаются за выполненные.

## 1. Главное решение

**Следующий приоритет — вертикальный запуск настоящего `Game`, а не завершение всех эффектов по очереди.**
Рендер уже достаточно развит, чтобы заняться startup, ресурсами, streaming и управлением.
Но GPU-фикстура с настоящим `gfx::Frame` ещё не равна запущенной игре.

Вести одновременно две критические линии:

1. **Game bring-up:** настоящее окно → загрузка ресурсов → меню → мир → управление → сохранение.
2. **Win7 qualification:** toolchain/std/CRT/ISA → PE32 → окно/WGL/input → тот же игровой artifact на целевой ОС/GPU.

Не ждать готовности облаков, panorama, HDR или всех специальных материалов, чтобы соединять игру.
Не ждать завершения Game bring-up, чтобы впервые проверять Win7: ОС может оказаться независимым блокером.
Современный wgpu-путь должен продолжать работать после каждого PR.

Номера ниже — **условные ID будущих PR**, а не уже открытые GitHub PR/issues. Этот план не создаёт PR,
не меняет чужие ветки и не утверждает, что исполнители уже назначены.

## 2. Что считать «запускаемым»

| Рубеж | Что реально должно работать | Что ещё не доказывает этот рубеж |
|---|---|---|
| **B0 — текущий baseline** | Native GL2 renderer, ресурсы и поддержанные stages настоящего Frame проходят readback-тесты | Нет обычного Game startup, интерактивного Linux окна, игрового ввода и Win7 runtime qualification |
| **B1 — интерактивный bring-up** | Настоящий Game открывает видимое окно, показывает загрузку/меню, загружает подготовленный мир, принимает клавиатуру/мышь, рисует мир и корректно закрывается | Не вся игра, не release; допускаются явно согласованные ограничения визуального профиля |
| **B2 — первый playable Linux/llvmpipe** | Меню → создание/загрузка тестового мира → ходьба/обзор → выбор/ломание/установка блока → предмет/рука/инвентарь/HUD → сохранение → повторная загрузка → выход | Не подтверждает Win7 или X1650; не обещает полную визуальную эквивалентность |
| **B3 — первый playable целевого устройства** | Тот же сценарий B2 реально выполнен на Win7 SP1 x86 + X1650 с аппаратным ICD; есть portable artifact и отчёт ограничений | Не означает полное покрытие всех миров/материалов, packs, mods и эффектов |
| **B4 — расширение и стабилизация** | Облака/panorama/спецматериалы, расширенные сценарии, длительная работа, измеренный performance | Не блокирует начало B1/B2; состав B3 явно фиксируется до выпуска |

**В B1/B2 игра — существующий Game и симуляция**, а не отдельная «мини-игра» или повторная реализация
движения/инвентаря. Для первого запуска можно дать прямой запуск существующего тестового мира, но он
не заменяет меню и создание мира в B2. Статический фон меню допустим только как явная часть legacy-профиля;
меню, текст и действия должны быть настоящими. Полная panorama — отдельная задача.

Временная Linux bring-up сборка может ещё содержать wgpu-зависимости, **не создавая wgpu device/surface
при выборе legacy**. Это ускоряет B1, но не проходит gate legacy-only artifact для B3.
Не объявлять такую промежуточную сборку Win7-совместимой.

## 3. Матрица функций и возможностей на текущем срезе

Обозначения: **Frame✓** — проверено через адаптер настоящего кадра, но не в обычном запущенном Game;
**native✓** — низкоуровневый контракт проверен; **частично** — основа есть, интеграция/qualification нет;
**нет** — нужного legacy пути пока нет. Состояние Win7/X1650 везде отдельно от Linux/CI.

| Функция / возможность | Сейчас | Для запуска / остаток | Рубеж / задачи |
|---|---|---|---|
| GL2/GLSL 1.10, typed VBO/EBO, bounded resources | native✓, llvmpipe 2.0/2.1 | Измерить реальные лимиты/ошибки ATI ICD | B3: O01, O02 |
| Resident terrain, cutout, translucent order | Frame✓ | Соединить настоящий streaming, CPU visibility, сортировку и unload | B1/B2: W01 |
| Camera-relative coordinates, projection, depth | Frame✓ | Подключить lifecycle окна/размеров; тестировать в движении | B1: P01–P04, G01 |
| Общие lightmap, fog, darkness/night vision | Frame✓ | Передавать live game state, не отдельные тестовые значения | B1/B2: G01, T02 |
| HUD, текст, paging, glyph patches, GUI models/glint | Frame✓ | Loading/menu/inventory используют общий facade; reset font cache и reload | B1/B2: A01, U01 |
| Entity/item/particle, hand, glint, static snapshots | Frame✓ для поддержанных материалов | CPU pose route, реальные producers и полный список common materials | B1/B2: E01, M01 |
| Камерные screen effects | Frame✓ | Проверить реальные причины эффектов и восстановление после resize/reload | B2: G01, T02 |
| Lightning, additive geometry, dragon rays | Frame✓ | Реальные producers; отдельные остальные спецматериалы не считать покрытыми | B2/B4: G01, M01, V03 |
| Rain/snow | Frame✓ | Реальный weather producer и reload ресурсов | B2: G01, A01, T02 |
| Atmosphere, sunrise, sun/moon, stars, End sky, dark disc | Frame✓ | Реальные ресурсы/состояние мира; не требует завершения облаков | B1/B2: A01, G01 |
| Окно WGL: ownership/events/resize/present | частично, native✓ | Game loop, относительная мышь, bindings, focus/minimize; Win7 runtime не проверен | B1/B3: P01, P02, P04, O02 |
| Linux видимое окно и native events | нет | EGL сейчас pbuffer, OSMesa — offscreen; нужен интерактивный путь | B1/B2: P03 |
| Backend selection в Game, loading renderer | нет | Game хранит конкретный `Option<gfx::Renderer>`; создание/поля завязаны на wgpu | B1: C01, R01–R03, G01, U01 |
| CPU resource bundle и реальный atlas upload | частично | Реальные pack layers → terrain pages; HUD/glint/animations/reload без wgpu handles | B1/B2: A01, A02 |
| CPU visibility, uploads, edits, unload | частично | Low-level API есть; интеграции Game streaming с legacy ещё нет | B1/B2: W01 |
| Keyboard/text/mouse → игровые действия | частично | Native events не являются gameplay bindings; Game handlers используют winit | B1/B2: P01, P02, P04 |
| Block outline / fishing/debug lines | нет, запрос кадра отклоняется | CPU/GL2 strokes; outline нужен для нормального взаимодействия | B1/B2: V01 |
| Breaking/crumbling overlay | не считать поддержанным | Сверить producer/material и реализовать cracks/blend/depth | B2: M01, V02 |
| Clouds, world-border visuals, portal/beacon/swirl/прочие спецматериалы | не покрыты либо отклоняются | Инвентаризация по кодам; разные независимые PR | B3/B4: V03–V05 |
| Title panorama, menu/post blur | отклоняются | Для B1/B2 явный static-menu profile; полноценные эффекты позже | B1: U01, F01; B4: V06 |
| GPU skinning, HDR/PBR/packs/compute/LOD/mirrors/extras | не входят в первый legacy milestone | CPU posing и явный capability gate; не создавать modern resources | B1–B3: E01, F01, D01 |
| Сеть/TLS/audio/controllers/narrator | игровая логика существует; legacy Win7 не квалифицирован | Не переписывать; аудит импортов/ISA и включение по одному | B3/B4: O01, O03 |
| Save/reopen мира и завершение | в существующей игре есть; legacy E2E нет | Регрессионный сценарий с реальным save/reopen, без потери данных | B2/B3: T02, O02 |
| Portable Win7 PE32 game artifact | нет | std/toolchain, dependency closure, CRT/PE/ISA, resources/run instructions | B3: D01, O01–O03, Q01 |

Проверенная база: [own CI 41811072027](https://github.com/lukintrees/horizon/actions/runs/41811072027)
на `d760a9a`: три jobs SUCCESS, **15 actual-game GPU fixtures**, full-client check/Clippy,
**3×26 independent GL readbacks**, **39 Windows x86 workspace tests** и ordinary probe linking.
Локально: **40 standalone Rust tests**, host/i686 Clippy, **642 Python tests, один skipped**.
Это **не** полный Game E2E и **не** запуск на Win7/X1650.

## 4. Минимальные контракты, позволяющие распараллелиться

Нужен один короткий contract PR **C01**, а не многонедельный универсальный renderer framework.
Зафиксировать эти границы до массовых изменений центральных файлов:

| Контракт | Обязательное содержание v1 | Чего не делать |
|---|---|---|
| **Renderer facade** | Выбор Modern/Legacy; menu/world render; upload/replace/remove section; resource reload; size/resize; present outcome; capabilities; screenshot/result/error | Не экспортировать `wgpu::Device/Queue/Surface`, GL handles или публичные backend-поля в Game |
| **Frame contract** | Общие CPU geometry/camera/HUD/environment данные; отдельно modern-only payloads; прежний `gfx::Frame` может временно остаться alias/re-export | Не вводить второй самостоятельный mesher/gameplay model; не копировать Frame в каждом adapter |
| **Resources** | Stable sprite IDs и layout generation; CPU RGBA layers/HUD/glint/animations; ownership/limits; reload completion/error | Не использовать wgpu `PreparedTextures` как общий GPU API; не держать две полные копии atlas постоянно |
| **Platform/loop** | Window size/title/events, pointer mode, monotonic scheduling, resize/minimize/close/focus; present result | Не переводить native events обратно в фиктивные winit events с поддельными IDs; не делать два конкурирующих loop |
| **Capabilities/profile** | Реальные capabilities отдельно от выбранной policy; negotiation до geometry generation; explicit errors и локализованные настройки | Не заменять producer fields пустыми внутри renderer; не выдавать обычный wgpu OpenGL за GL2 |
| **Streaming** | Section key + world/resource generation, mesh revision, visible order, replace/unload, bounded upload queue | Не принимать stale job после teleport/reload/unload; не отключать CPU simulation при GPU backpressure |

Начальные `render_contract`, `resource_contract`, `platform_contract`, borrowed `renderer_facade`
и R01a `frame::Frame`/CPU DTO уже добавлены в [C01 v1](legacy-gl-c01.md); `legacy_resources`,
visibility/resource descriptors и owning facade миграция ещё не реализованы. C01 утверждает имена/владение; исполнители
не должны независимо придумывать несовместимые версии этих типов.

Зацепка `game/event_loop.rs::EventLoopCtl` уже принимает нейтральный `WakePolicy` вместо winit
`ControlFlow`; native handlers/window ownership ещё требуют P01–P04. Новая симуляция не нужна.
`Frame`, `SkinFrame`, lightning inputs и shader-pack input/span DTO теперь объявлены в общем
wgpu-handle-free `frame/` и совместимо re-exported из прежних `gfx::*` путей. Modern GPU state остался
в `gfx`; одного переноса DTO для legacy-only dependency graph всё равно недостаточно.

Решение v1 для ownership: сохранить существующий native owner окна/context на legacy-пути,
modern window/loop оставить winit-adapter. Общий Game не должен требовать `Arc<winit::Window>`
для создания legacy renderer или получать фиктивное окно. GL calls остаются на потоке текущего
контекста; FFI — в native crate, без снятия `forbid(unsafe_code)` в игровом/render слое.

## 5. Backlog будущих PR

Приоритет: **P0** — критический путь B1/B2 или независимый риск Win7, начать сейчас;
**P1** — B3/расширение либо параллельная работа без задержки bring-up;
**P2** — после первого playable. Зависимости — по завершённым контрактам/кодовым PR, не по календарю.
Каждая строка — work item: при размере свыше примерно 500 строк изменения делить на несколько небольших
PR `a/b/c`, сохраняя критерий приёмки строки и зелёный modern путь.

| ID | Приоритет | Работа и основной участок | Зависит от | Проверяемая приёмка |
|---|---|---|---|---|
| **C01** | P0 | Инвентаризация всех вызовов/полей renderer в Game; контракты §4, минимальные типы/скелеты, список запрещённых прямых доступов | — | Согласованный API и карта call sites; не осталось «потом решим», кто владеет окном/ресурсами/sections; default build не меняет поведения |
| **CI01** | P0 | PR-triggered/reusable и aggregate CI, scope/ownership gate, modern/legacy/GL2/x86 matrix; [спецификация](legacy-gl-integration.md#6-ci01--praggregate-ci-для-параллельной-разработки) | design сразу; C01 для policy/contract paths | Task diff с facade/manifest/lockfile/workflow блокируется; exact base/head/merge SHA и expected jobs проверены; Win7 hardware gate отдельно; required checks не пропускаются. CI01a/b/c добавлены; CI01d и admin/bootstrap gates остаются открыты |
| **R01** | P0 | Вынести CPU Frame/visibility/resource descriptors из monolithic gfx; совместимые re-exports | C01 | Общий слой компилируется без wgpu handle types; старые CPU/GPU тесты и public call sites сохраняются |
| **R02** | P0 | Adapter существующего modern Renderer за facade; заменить прямые поля getter/commands. R02a–g уже перевели screenshot request/result, drawable size, surface handover, GPU facts/present mode и terrain upload/clear/unload/resize на facade; native terrain учитывает dimension min_y; owning migration ещё нет | R01 | Обычный modern loading/menu/world/reload продолжает работать; новый API проверен на существующем backend |
| **R03** | P0 | Adapter `TerrainHudRenderer` за тем же facade, native factory, render/present outcomes, budgets | R01, A01 | Общий вызов создаёт legacy, принимает resources/frame/sections; не вызывает wgpu initialization; ошибки не маскируются |
| **A01** | P0 | Общий CPU resource bundle и legacy atlas preparation из настоящего pack: `PackResources`, `PreparedTextures`, `HudAtlas`, sprites | C01 | Реальные layer IDs корректно превращаются в страницы; fonts/glint работают; ограниченные allocations; новые layout generations |
| **A02** | P0 | Жизненный цикл ресурсов: animated textures, dynamic patches, font-cache reset, reload/rollback/context recreation | A01, R03 | Реальный pack reload и смена языка/шрифта не оставляют старые UV/glyph caches; stale jobs отклонены; memory bounded |
| **P01** | P0 | Нейтральные window/input/loop events и scheduling; адаптер нынешнего winit пути | C01 | Modern loop не регрессирует; общие handlers больше не требуют native/winit IDs или ControlFlow |
| **P02** | P0 | Win32/WGL platform adapter: raw/relative mouse, capture, focus, DPI policy, resize/minimize/close | P01 | Mouse-look без упора в границы; Alt-Tab и lost-event reset не залипают; resize/present/recreate проверены в настоящем окне |
| **P03** | P0 | Видимый Linux window path с desktop GL2/llvmpipe, события и pointer mode | P01 | Интерактивное окно под X11 + llvmpipe, не pbuffer; тот же Game/platform contract. Для первого пути выбрать GLX либо EGL-on-X11, не реализовывать сразу все display systems |
| **P04** | P0 | Общий input → existing gameplay bindings/text/UI, tick/redraw/pause/minimize policy | P01; platform adapter для E2E | WASD/mouse-look/click/scroll/Escape/text работают в Game; focus loss сбрасывает held state; нет busy-spin в minimized |
| **F01** | P0 | Legacy profile + negotiation до построения кадра; graphics settings/CLI/config diagnostics | C01 | Явный отдельный legacy выбор; CPU poses; static menu и отключённые необязательные визуальные extras документированы; save settings не ломают modern profile |
| **E01** | P0 | CPU pose producer route, item/hand/static models; не только `skin=None` | R01, F01 | Реальные обычные entities видимы и анимируются; modern GPU posing продолжает работать; legacy не создаёт GPU skin store |
| **W01** | P0 | Game world streaming → legacy upload/unload, CPU frustum/cave visibility, quad/section transparency sort, bounded queues | C01, R03 | Мир не только один раз загружается: ходьба, block edits, teleport, unload/reload; generations/revisions защищают от stale meshes; VRAM/RAM не растут без границ |
| **U01** | P0 | Loading/title/menu/inventory через facade, отдельный menu request и явный static background v1 | R02, R03, F01 | Настоящие loading/title/world-selection/inventory screens; Unicode/glyph patches; не обход `title=true` тихим обнулением флага |
| **M01** | P0 | Таблица material codes всех реальных producers + Frame/HUD feature audit на стартовом сценарии | можно аудит сразу; contract C01 | Для каждого кода: producer, shader policy, supported/fallback/blocker, тест; выявлены hard blockers обычного gameplay, не только тестовых quads |
| **V01** | P0 | Block outline и обычные lines: CPU/screen-space strokes, depth/on-top order; новый модуль, тонкое wiring | C01, M01 | Смотреть на блок/держать удочку не вызывает unsupported frame; корректные ширина/depth/FOV, без обязательных wide GL lines |
| **V02** | P0 | Breaking/crumbling overlay: projected sprite repeat, blend/depth/cutout по producer contract | M01; R03 для E2E | При удержании mining видны cracks/stages, после слома исчезают; установка/ломание не нарушают terrain/HUD |
| **G01** | P0 | Единственная интеграционная ветка Game startup/factory/loop/draw: `entry.rs`, `game.rs`, build/loading renderer | R02, R03, W01, E01, P04, F01, U01; выбранная P02/P03 | Настоящий Game доходит до world render, не создавая wgpu surface/device в legacy; ownership/present/errors/exit корректны |
| **T01** | P0 | E2E harness/log stages/repro resources и сценарий запуска, независимый от текущего renderer tests | подготовку начать сразу, API C01 | Чётко различает initialization/menu/world/input/save checkpoints; запускает только fresh homes; падает при тихом fallback к wgpu |
| **T02** | P0 | Linux playable acceptance B2 на реальном Game и persistence | T01, G01, W01, E01, A02, V01, V02 | Полный сценарий §7 с save/reopen; no-assets-in-Git; видео/скриншоты и machine-readable result, не только success exit |
| **D01** | P0 | Legacy-only dependency closure: manifests client/render/lib/input/HDR/mod hooks, feature unification/build scripts | C01; кодовый переход по R01/R02/P01 | Legacy-only release не тянет обязательные wgpu/DX12/Vulkan/GPUI/modern window paths; отдельно проверены нормальная и legacy-only сборки |
| **O01** | P0 | Win7 x86 toolchain/std/SDK/CRT/ISA и минимальный OS probe, import audit | —, использует существующий native crate | Воспроизводимый pinned build; PE32/динамические imports/ISA отчёт; **реальный** Win7 probe либо явно незакрытый hardware gate |
| **O02** | P0 | Целевой Win7/X1650 стенд: capabilities/driver, window/input/swap, затем Game B2 | O01, P02; G01/D01/T02 для Game | Чистый запуск с vendor ICD, реальные GL limits, полный B2 и ограничения; ordinary Windows CI не подменяет результат |
| **O03** | P1 | Runtime subsystems на Win7: allocator/threads/files/TLS/network; затем audio/controllers/narrator по одному | O01, D01 | Нет обязательного Win8/10 API/x64 DLL/неподходящего ISA; выключенные optional subsystems явно отражены в artifact |
| **Q01** | P1 | Portable artifact, ресурсы/подготовка из своей установки, запуск/rollback, capability report | D01, O01; B2/B3 для соответствующей маркировки | На чистой машине понятны prerequisites/CRT/resource path; одно воспроизводимое действие запуска; нет bundled Mojang assets или обязательного GPUI launcher |
| **V03** | P1 | Спецматериалы по каталогу: portal/beacon/swirl/dissolve и прочее — отдельные PR | M01, C01 | Каждый материал имеет реальный producer fixture, policy/blend/depth test и область поддержки; common hard blockers повышаются в P0 |
| **V04** | P1 | Clouds в отдельном модуле | C01, R01 | bounded geometry, fog/order/culling, реальные Frame inputs; не блокирует B1/B2 при явном clouds-off profile |
| **V05** | P1 | World-border visuals | C01, M01 | Видимый border/fog/order; физическая граница/симуляция никогда не выключается ради рендера |
| **V06** | P2 | Full panorama, blur/post effects и остальной UI parity | U01, C01 | Полный эффект или явная capability error; не ломает простой стартовый menu path |
| **Q02** | P1 | Performance/memory/soak и восстановление | B1 для начала замеров, B2/B3 для gate | Frame time p50/p95, peaks CPU/GPU/upload queue, длительная ходьба/reload/Alt-Tab, no leaks; отдельные llvmpipe и ATI результаты |

Приёмка adapters до G01 использует настоящие CPU producers и native/contract fixtures;
live Game часть критериев совместно закрывается G01/T02. Это не повод называть mock успешным
игровым запуском и не требование построить полноценный Game E2E до существования startup wiring.

**M01 — обязательная ранняя работа:** существующий converter не равен всем материалам игры.
Если каталог или E2E выявит регулярно генерируемый неподдержанный material, нужный PR поднимается в P0
до B2. Не делать «поддержку» через скрытый пропуск этой геометрии.

## 6. Что можно распараллелить раньше всего

### Сразу, не дожидаясь большого refactor

| Дорожка / условный владелец | Первая работа | Почему независима |
|---|---|---|
| **Integrator / contracts** | C01, карта Game call sites; затем R01/R02 | Владеет общими API и сериализует wiring |
| **OS/build** | O01 + начальный аудит D01/O03 | Не требует нового renderer facade, проверяет отдельный риск ОС |
| **Resources** | Аудит/CPU fixtures A01, схема IDs/reload | Читает текущие CPU ресурсы; финальные типы согласовать с C01 |
| **Platform/input** | Аудит P01/P02/P03 и контракта events | Имеются native WGL events и существующий EventLoopCtl; можно выявлять связи сейчас |
| **QA/E2E** | T01, стартовый тестовый сценарий, asset/home isolation | Заранее фиксирует, что считать игровым запуском, а не renderer demo |
| **Render/materials** | M01; затем V01/V02 по контрактам | Аудит producer codes и CPU math не требует startup интеграции |

Это **не** требование шести исполнителей. При 3–4 людях роли объединяются, но сохраняются ownership
и DAG. Нельзя назначить независимыми задачами два одновременных переписывания `game.rs`.

### После C01 — первая широкая волна кодовых PR

Параллельно: **R01**, **A01**, **P01**, **F01**, **V01/V02 CPU modules**, **O01**, **T01**, **CI01**.
D01 начинает с измеримого dependency inventory/feature design, а не массового `cfg` всего workspace.

После R01/A01/P01: **R02**, **R03**, **P02**, **P03**, **E01**, **U01** (по своим зависимостям).
Платформенные adapters и renderer adapters не должны тянуть друг друга через concrete window/device types.

После adapters: **W01**, **P04**, **A02** и интеграционный **G01**. Интегратор может готовить wiring
по уже зафиксированным интерфейсам, но merge приёмки G01 требует всех зависимостей из таблицы.
Затем **T02** закрывает Linux playable; одновременно **D01/O02/O03/Q01** доводят целевой artifact.
Остальные visual PR идут в стороне от критического пути, если M01 не выявил hard blocker.

### Критические пути

```text
C01 → R01 → R02 ──────────────────────────────┐
C01 → A01 → R03 → W01 / A02 ──────────────────┤
C01 → P01 → P03 (Linux) / P02 (Win32) → P04 ──┤→ G01 → T02 → B2
C01 → F01 → E01 / U01 ────────────────────────┤
M01 → V01 / V02 ──────────────────────────────┘

O01 ────────────────────────────────┐
C01/R01/P01 → D01 ──────────────────┼→ O02 + Q01 + B2 scenario → B3
P02 + необходимые O03 subsystems ───┘
```

Граф — сводка. При различии с графом точные зависимости/гейты берутся из таблицы PR;
B2 требует W01/A02/E01/V01/V02, даже если первый черновой G01 уже открыл окно.

## 7. Сценарий приёмки, общий для QA и исполнителей

1. Все процессы получают fresh scratch `HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME`,
   `XDG_CONFIG_HOME`; launcher self-update выключен. Первые no-audio artifact/tests явно так помечены.
   Не запускать тесты на реальных player saves. Сохранить конкретный legacy backend/profile в отчёт.
2. Запуск не создаёт wgpu device/surface; лог показывает выбранный backend, GL/GLSL/vendor/limits,
   resource generation и этапы resource-ready/menu-ready/world-ready. Нет молчаливого modern fallback.
3. Настоящее loading/menu: ввод имени/Unicode, выбор/создание мира, корректный HUD scale.
4. В мире: ходьба/прыжок/обзор/click/scroll/Escape; resize, minimize/restore, focus loss/Alt-Tab.
   Проверить отсутствие stuck input и busy-loop. Движение камеры вызывает streaming, а не один static draw.
5. Выбор блока, cracks, ломание, установка; обновление mesh и инвентаря/руки. Обычная entity видима,
   а не пропущена после отключения GPU skinning. Вода/стекло/cutout/HUD не исчезают.
6. Save → закрытие → повторное открытие **того же тестового save**: изменения блоков/инвентаря сохранены.
   При необходимости тестировать pause/save без menu blur через явно выбранный профиль.
7. Resource reload/переключение языка и продолжение игры: нет stale atlas/glyph/section generation.
   Заложенные budgets не превращаются в бесконечный рост RAM/VRAM/очереди.
8. На Win7 повторить тот же сценарий, записать реальные driver/CPU/VRAM/CRT и imports. В label B3
   нельзя подставить результаты Windows-latest или Linux. Если стенда нет — gate остаётся открытым.

**Отдельный build gate O01/D01:** проверить `i686-win7-windows-msvc` у выбранного pinned toolchain,
наличие std artifacts; при необходимости — воспроизводимый nightly/rust-src + build-std путь.
У Game MSRV 1.90/edition 2024: нельзя просто понизить Rust ради Win7. Обычный
`i686-pc-windows-msvc` и один успешно запущенный probe на Windows-latest не доказывают совместимость.
Проверять imports и динамическую загрузку всех DLL, CRT deployment, allocator и ISA/CPUID;
модель GPU не определяет CPU. Проверить 32-bit address-space peaks отдельно от RAM и VRAM.

Не задавать сейчас выдуманный FPS для X1650. В Q02 сначала замерить разрешение, render distance,
frame-time p50/p95, memory peaks и upload pressure; затем утвердить бюджет и критерий выпуска.
Поломка Win7 imports или нехватка address space имеет приоритет над косметическими эффектами.

## 8. Как не превратить параллельные PR в merge bottleneck

- **Один владелец интеграции — координирующий Arena.ai Agent `lukintrees`:** `game.rs`, `entry.rs`, общий facade/frame contract и центральные manifests.
  Остальные PR добавляют отдельные modules/adapters/tests; общий wiring интегратор переносит малыми изменениями.
- **Низкоуровневый GL coordinator/shaders — тоже общая зона.** V01–V06 заранее согласуют pass slots,
  blend/depth/sampling contracts; не редактируют одновременно весь `renderer.rs` и shader dispatcher.
  CPU preparation и independent fixtures можно ревьюить раньше последнего wiring patch.
- **Один API PR — несколько adapters:** не выносить тысячу GPU методов в общий trait. Общими остаются
  Game-facing операции; modern diagnostics/pack internals остаются у modern adapter.
- **Manifest/lockfile coordination:** D01 готовит dependency inventory/feature requests; **центральные edits делает только интегратор**, остальные заранее перечисляют запросы.
  Не обновлять dependency versions «заодно» и не удалять lockfiles/`CLAUDE.md`.
- Каждый PR указывает ID, Depends-on, own scope/files, negotiated capability, тесты, признаки готовности
  и ожидаемый способ merge. Небольшие dependency-stacked PR допустимы; не копировать реализации в ответвления.
- **Green after each merge:** modern compile/tests, affected legacy CPU/native/GPU checks. У render PR —
  реальные readbacks и invalid-input/resource tests; у platform/build — реальные OS/runtime gates отдельно.
- Существующий `.github/workflows/legacy-gl-branch.yml` привязан к текущей ветке. Он **не обеспечивает**
  CI будущих параллельных PR автоматически. Эту проблему закрывает **CI01 (P0)**: reusable/PR-triggered
  matrix для согласованных task PR и aggregate integration head; scope/ownership gate и проверки
  точного кандидата. Scope/matrix теперь исполняются в CI01a/b/c; human dependency review и bootstrap/protection
  остаются обязательными. CI01d будет подключён после реального Game bring-up.
- App/asset preparation не публикует Mojang assets/jars/decompiled files; fixtures — собственные synthetic
  данные либо разрешённая локальная подготовка. Пользовательские строки — через существующую локализацию.

Шаблон work item / PR:

```text
Task-ID: M01a
Variant: A
Contract-Version: 1
Цель / рубеж:
Depends-on:
Owner + reviewer:
Base / head / exact tested SHA / contract version:
Central files changed: none (shared changes requested from integrator):
Изменяемые modules / общие файлы, которые НЕ трогаем:
Contract version / capability:
Acceptance: наблюдаемый игровой или runtime результат:
Tests: modern + legacy + platform, что ещё не проверено:
Limits / failure behavior / reload + ownership:
Merge order / интеграционный patch:
```

## 9. Что делать следующим, в порядке отдачи

1. **C01 + CI01 + T01:** зафиксировать API/владение, проверки параллельных PR и первый реальный acceptance scenario.
2. **Сразу параллельно O01, M01 и resource/platform audits.** Не откладывать Win7 и каталог материалов.
3. **Первая кодовая волна R01/A01/P01/F01**, затем adapters. Не продолжать только декоративные pass'ы.
4. **G01 + W01 + E01 + U01/P04:** вывести настоящий Game к интерактивному миру.
5. **V01/V02 + A02 + T02:** закрыть первый playable B2 и persistence.
6. **D01/O02/O03/Q01:** получить и реально квалифицировать целевой B3 artifact; вести это параллельно,
   а не после завершения всей визуальной совместимости.

Никакие сроки B3 не обещаются до доступного Win7/X1650 стенда и результата O01. На старте лучше
расставить владельцев шести дорожек и согласовать C01, чем открывать десятки пересекающихся PR в `game.rs`.


### Продолжение U01a / CI cleanup (2026-11-05)

R02g проверен restored Actions на `904e30d`/`0dbd7bd` (run IDs и границы доказательства —
[C01 §6](legacy-gl-c01.md#6-r02g-проверка-и-открытые-gates-2026-11-05)). По запросу пользователя
лишний CI архивирован; focused workflow сохраняет modern/full-client, native GL2 и x86 gates.

Первый U01 этап: общий typed `MenuFrame`, явный static RGBA background, modern panorama по умолчанию,
legacy capability rejection для panorama и семь loading/save/no-resource menu submissions. Readback fixture
проверяет, что menu не уничтожает resident world и invalid input не стирает прошлый кадр.
Это submission seam, **не закрытие U01/B1**: главный title/world path и owning Game пока modern;
F01 profile negotiation, R03 resources, native factory/startup, screenshot completion и настоящий
interactive menu/world остаются необходимы. Default modern path не переключается на legacy автоматически.


### Продолжение U01b (2026-11-05)

Native screenshot commands фасада теперь выполняются после успешного world/menu draw до present:
SDR PNG native resolution и centre-cropped icons, одноразовые pending/result transfer, bounded CPU
readback и явные I/O/budget errors. Современный screenshot path не изменён. Supersampling/upscaling
не подменяются низким качеством: возвращается ошибка; F01 должен выбрать native-resolution F2 policy.
Подробности/лимиты и проверка — [C01 §8](legacy-gl-c01.md#8-u01b-bounded-native-screenshot-completion-2026-11-05).
Это закрывает initial native screenshot completion seam, но **не U01 целиком и не B1**:
Game пока modern-owned, interactive native menu/startup и resource/profile integration ещё нужны.
