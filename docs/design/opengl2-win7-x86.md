# План: OpenGL 2.0/2.1, Radeon X1650, Windows 7 x86

Статус: **план; реализованы Linux probe и WGL-прототип, не поддержка игры**. Дата: 2026-11-04.
Основание: текущий checkout `8dbbde43501924d16497d251e6e9d08f812eafcd`.
Цель — запуск самой игры на Windows 7 SP1 **32 bit** с AMD/ATI Radeon X1650
через установленный драйвер производителя. Linux + Mesa llvmpipe — основной
автоматизированный стенд разработки, но не доказательство совместимости с ATI.

## 1. Решение и границы

Не переписывать игровую логику, протокол, миры и генератор. Добавить отдельный
legacy OpenGL renderer и Win7-x86 сборку, сохранив существующий wgpu renderer
для современных компьютеров. Полная замена рендера во всех сборках не нужна.

Базовый контракт: **desktop OpenGL 2.0 + GLSL 1.10**. OpenGL 2.1 + GLSL 1.20 —
дополнительный проверяемый режим, не обязательный минимум. Это не OpenGL ES 2.0.
Версия API и расширения конкретного Win7-драйвера X1650 должны быть измерены;
название видеокарты само по себе не гарантирует конкретный набор возможностей.

Первый playable milestone: меню, загрузка мира, ходьба, блоки, освещение,
прозрачность, базовые сущности, рука/предмет, HUD, инвентарь, сохранение и выход.
Демонстрация нескольких текстурированных чанков — только промежуточный этап.

Не включать в первую версию: HDR, PBR, shader packs, compute, GPU skinning,
frame generation, дальние LOD, отражения, splitscreen, GPUI launcher и native
code mods. Отключённые эффекты не должны менять симуляцию или содержимое мира.
Упрощения изображения документировать; совместимость не означает полную
визуальную идентичность современному renderer.

## 2. Что обнаружено в репозитории

| Участок | Текущее состояние / необходимое изменение |
| --- | --- |
| `crates/horizon-client-render/src/gfx.rs` и `gfx/` | Большой конкретный `Renderer`, wgpu device/queue/surface, uploads, passes, GPU culling. Выделить backend-neutral фасад и данные кадра. |
| `crates/horizon-client-render/src/terrain.wgsl`, `entity.wgsl`, другие WGSL | Storage buffers, texture arrays, современная модель шейдеров. Написать небольшие собственные GLSL 1.10 программы, не рассчитывать на автоматический перевод WGSL. |
| `mesher.rs`, `mesher/packed.rs`, `gfx/upload.rs` | Сохранить правила meshing и CPU-освещения; добавить преобразование packed quads в legacy вершины/индексы. |
| `hud.rs`, `screen_draw.rs`, `font.rs`, `sprites.rs`, `mipmap.rs` | Переиспользовать CPU-геометрию и ресурсы; заменить GPU upload/draw, проверить atlas addressing. |
| `crates/horizon-client/src/game.rs`, `game/` | Создание renderer, его методы/поля и построение кадра связаны с текущим API. Найти и убрать прямые wgpu-зависимости из общего пути. |
| `crates/horizon-client-render/src/graphics.rs`, `graphics_ui.rs` | Явный выбор legacy backend, capability gating и безопасный preset. Существующий `OpenGl` не переименовывать молча в GL2. |
| `crates/horizon-client{,-render,-lib}/Cargo.toml`, `horizon-hdr` | Аудит графа зависимостей и feature unification: legacy-only executable не должен требовать wgpu/Vulkan/DX12. |
| `crates/horizon-client-input`, audio, narrator, allocator, networking | Независимые от GL риски Win7 и 32-bit; проверять каждый включённый runtime-компонент. |
| `launcher/Cargo.toml`, `.github/workflows/game-build.yml`, packaging | GPUI launcher — отдельная задача. Нужен отдельный x86 portable artifact и способ прямого запуска. |

У render crate стоит `#![forbid(unsafe_code)]`: не снимать запрет ради GL.
FFI, загрузку GL-функций и владение контекстом изолировать в небольшом новом
низкоуровневом crate с безопасным публичным API и документированными invariants.

На момент аудита открыт [draft PR #1](https://github.com/lukintrees/horizon/pull/1)
с CPU atlas/terrain prototypes и GLSL 1.10. Перед реализацией согласовать работу
с ним: переиспользовать проверенные части, не создавать параллельный terrain
backend. По описанию PR контекст, playable integration и runtime-тесты ещё отсутствуют.
Этот документ — план по текущей базе, не утверждение о качестве кода того PR.

## 3. Два независимых блокера: ОС и GPU

### 3.1. Windows 7 x86 — проверить до большой переделки рендера

1. Зафиксировать edition/MSRV: workspace использует Rust edition 2024, ключевые
   client crates требуют Rust 1.90. Простое снижение Rust до старого Win7-совместимого
   toolchain не является решением: сломаются язык и зависимости.
2. Проверить доступность/статус `i686-win7-windows-msvc` у выбранного toolchain
   (`rustc --print target-list`), наличие std artifacts. Если это tier-3 путь без
   готового std — воспроизводимо собрать std подходящим nightly + rust-src и
   `-Z build-std`; зафиксировать версии. Не обещать, что `rustup target add` сработает.
   Обычный `i686-pc-windows-msvc` не считать гарантией Win7.
3. Собрать маленький PE32 probe: старт, heap, threads, файл, таймер, окно, WGL,
   shader compilation, swap buffers. Зафиксировать SDK, linker и CRT deployment.
4. На Windows 7 SP1 x86 проверить запуск с нуля и imports **всех** DLL/EXE:
   отсутствие обязательных Win8/10-only API, x64 DLL, неподходящего CRT и ISA.
   Проверять и динамически загружаемые API, не только PE import table.
5. Последовательно добавить окно/input, allocator, файлы, сеть/TLS, ресурсы,
   звук, narrator, controllers. Спорные подсистемы сначала compile-time исключить,
   затем вернуть по одной. `--no-default-features` сегодня отключает audio/controllers,
   но само по себе не создаёт legacy-граф зависимостей.
6. Проверить CPUID целевого процессора: X1650 не определяет CPU/ISA. Не использовать
   `target-cpu=native` для распространяемой сборки; проверить нативные библиотеки
   и runtime dispatch (в том числе allocator) на старом процессоре.

**Gate A:** PE32 probe реально стартует на Win7 x86, показывает GL-треугольник,
а минимальная сборка общих систем загружает ресурсы. Linux cross-build и Wine
полезны, но не закрывают этот gate. Без Win7 стенда статус остаётся «не проверено».

### 3.2. Реальные возможности драйвера

Probe должен записывать vendor/renderer, GL/GLSL version, расширения, лимиты
texture size, texture units, vertex attributes, uniforms/varyings, shader logs,
pixel format/depth bits, способ загрузки функций, версию/разрядность драйвера.
Начинать с 2D RGBA8 POT текстур и обычного depth buffer окна.
Не предполагать объём VRAM по названию X1650: у разных плат он различается.
`Microsoft GDI Generic` означает отсутствие требуемого аппаратного ICD, а не
поддержку GL2. Выдавать понятную диагностическую ошибку вместо падения.

## 4. Контракт legacy renderer

| Современный механизм | Обязательная legacy замена |
| --- | --- |
| WGSL / bind groups / UBO / SSBO | GLSL 1.10, небольшие обычные uniforms и vertex attributes |
| Texture arrays | Несколько страниц 2D atlas, draw batching по странице |
| Vertex pulling / packed GPU quads | CPU unpack при изменении mesh → VBO + EBO, `glDrawElements` |
| Indirect / multi-draw / instancing | CPU draw list и обычные draws; без обязательных instancing extensions |
| Compute culling / occlusion | CPU frustum + существующая CPU visibility logic; запросы occlusion необязательны |
| GPU pose / sprite animation / lightmap | CPU pose, CPU обновления небольших texture regions / lightmap |
| HDR / float targets / MRT / post chain | SDR RGBA8, обычный framebuffer окна, простой fog/blend |
| Современная синхронизация / timers | Ограниченные upload queues и CPU timings; sync/timer extensions не обязательны |

Дополнительные правила реализации:

- GL calls и создание/удаление ресурсов — только на потоке текущего контекста.
  Workers готовят CPU данные; bounded queue, лимит upload bytes/frame.
- VBO/EBO с явными attribute pointers; VAO, sampler objects и FBO не требуются.
  Offscreen FBO разрешить только отдельным проверенным extension path;
  основной запуск обязан работать без него. Screenshots — `glReadPixels` из окна.
- WGL: создать обычный legacy context через Win7 APIs, не требовать GL3 core
  profile или `WGL_ARB_create_context`. Загрузить функции через `wglGetProcAddress`
  с fallback к `opengl32.dll`, отфильтровать недопустимые sentinel pointers.
  На Linux — GLX либо desktop EGL, не GLES context. Библиотеку окна/контекста
  выбрать после probe; проверить winit/glutin Win7 dependency graph, не делать
  массовый downgrade winit без доказанной необходимости.
- GLSL 1.10: `attribute`, `varying`, `texture2D`, `gl_FragColor`; без uint/bitwise,
  `layout`, `texelFetch`, `gl_VertexID` и sampler-array dynamic indexing.
  Атрибуты назначать перед link, диагностировать compile/link failures.
- Камера/mesh: world coordinates хранить с текущей точностью CPU; в GL передавать
  camera-relative float coordinates. Отдельно исправить projection/depth convention:
  desktop GL clip-space Z = -w..w, нельзя слепо копировать wgpu projection 0..w.
- Atlas: лимит страницы из capabilities; POT sizing, padding/extruded borders,
  раздельные mip chains для sprites. Не переносить UV texture-array слоя напрямую.
  Проверить repeating UV, animated sprites, rotated faces и pack reload.
  Не вводить обязательную NPOT/mipmap комбинацию без capability-теста.
- Непрозрачные → alpha cutout (`discard`) → отсортированные translucent draws;
  для прозрачности depth test включён, depth write выключен. Зафиксировать
  winding, culling, blend factors и порядок hand/HUD. Не прятать всё стекло/воду.
- Фичи проверяются по реальным capabilities и контракту backend. Ни одна
  неподдерживаемая настройка не должна создавать modern resources «на всякий случай».
  UI-ошибки и названия настроек — через существующие translation keys.

## 5. Этапы и критерии готовности

Каждый пункт разбивать на небольшие PR, современный backend сохранять рабочим.
Названия новых crates, features и CLI ниже — проектные, пока их нет в коде.

| Этап | Работы | Проверяемый результат |
| --- | --- | --- |
| 0. Probes / platform audit | Win7 PE32/WGL probe; Linux llvmpipe setup; dependency tree и limits report | Gate A либо явный список блокеров; Linux triangle compile/link/draw/readback |
| 1. Граница backend | Выделить `RenderFrame`, handles/caps и фасад render/upload/resize/readback; закрыть прямой доступ к device/queue | Современный renderer проходит прежние тесты; общий game path не зависит от GPU API |
| 2. Legacy-only build | Небольшой GL FFI crate; native context; compile-time features, условные HDR/shaders dependencies | Legacy executable не линкуется с wgpu/Vulkan/DX12; окно, resize, clear и HUD quad |
| 3. Ресурсы и UI | Atlas pages, fonts, clipping/scissor, menus, inventory icons, resource reload | Title/options/loading/inventory читаемы, ввод работает, текстуры не протекают между sprites |
| 4. Terrain | Packed mesh conversion, VBO/EBO lifetime, uploads/removal, light/AO/tints/fog, cutout/water/glass | Контрольная сцена: камень, трава, листва, вода, стекло, факелы; day/night и block edits видимы |
| 5. Playable path | Entities/items, CPU poses, first-person hand, block entities, particles, sky/weather/overlays | Создать/открыть мир, ходить, ломать/ставить, взять предмет, сохранить, перезапустить, подключиться к серверу |
| 6. x86 budgets | Ограничить caches/workers/queues; eviction; fallback settings, diagnostics | Длительная сессия и переходы миров не исчерпывают адресное пространство; нет роста handles/GL objects |
| 7. Доставка | Win7 toolchain lock, PE/DLL audit, portable archive, инструкции ресурсов, CI | Чистая Win7 SP1 x86 + X1650 запускает artifact без SDK/Rust/современного launcher |

Ранний Win7 probe нужен **до** этапа 1; полный x86 build поддерживать по мере
каждого этапа, не переносить платформенные проблемы на конец проекта.

## 6. Память и стартовые настройки

32-bit Windows обычно даёт процессу около 2 GiB user virtual address space.
Не полагаться на `/3GB` или Large Address Aware; fragmentation, mappings,
CRT, драйвер и integrated server тоже расходуют пространство. RSS недостаточно:
измерять private/commit и virtual address usage, крупнейшую доступную область.

Начальные инженерные бюджеты (не измеренные свойства проекта):

- Нормальная сессия: целиться в private commit ≤ 1.2 GiB, отдельно контролировать
  VA и transient peaks; оставлять запас до практического предела процесса.
- GL textures + buffers: стартовый soft cap 64 MiB, менять только после измерений
  на конкретной плате. Учёт allocations приблизителен и не равен VRAM драйвера.
- Render distance 4 chunks, simulation distance 4 как стартовый preset;
  GPU LOD/shader packs/HDR выключены, standard-resolution textures, без MSAA,
  небольшое число workers. Проверить минимальный полный atlas и его размер:
  если не укладывается, использовать страницы/lazy residency, не удалять ресурсы молча.
- Bounded mesh/result queues, eviction далёких CPU/GPU meshes, никаких постоянных
  двойных полноразмерных atlas copies. Учесть expanded vertices относительно packed quads.
- Сначала multiplayer client (меньше нагрузки), затем single-player с integrated
  server. Не объявлять цель выполненной, если работает только подключение к серверу.

Для реальной X1650 предложенный performance target — 25–30 FPS при 800×600,
4 chunks в зафиксированной сцене без shader packs. Это ориентир, **не гарантия**:
CPU, RAM и VRAM пока неизвестны. Собирать p50/p95/p99 frame time, stutters,
upload/draw counts, память и время загрузки. FPS llvmpipe не предсказывает FPS ATI.

## 7. Как тестировать здесь: Linux + llvmpipe

В sandbox Debian 12 при первоначальном аудите не были найдены `rustc`, `cargo`, `glxinfo`,
`Xvfb` и GL/EGL библиотеки в `ldconfig`. Игровые prepared resources не обнаружены.
Это исходное состояние, **не успешный тест**; результат bootstrap ниже. Доступ к сети ограничен, поэтому
доставку Rust/Mesa/Xvfb через разрешённый канал или подготовленный образ включить
в этап 0; не обещать доступность apt/rustup downloads в этом окружении.

После подготовки образа нужны Mesa software DRI/GLX, Xvfb, mesa-utils, Rust,
C/C++ toolchain и platform headers. Сначала записать `glxinfo -B`, versions и
выполнить настоящий render/readback probe. Mesa renderer обязан содержать
`llvmpipe`: одного `LIBGL_ALWAYS_SOFTWARE=1` недостаточно.

Матрица запусков через Xvfb, например для оконного GLX probe (ещё не реализован):

```sh
LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
  MESA_GL_VERSION_OVERRIDE=2.0 MESA_GLSL_VERSION_OVERRIDE=120 \
  xvfb-run -a -s '-screen 0 800x600x24 +extension GLX' <legacy-probe>

LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
  MESA_GL_VERSION_OVERRIDE=2.1 MESA_GLSL_VERSION_OVERRIDE=120 \
  xvfb-run -a -s '-screen 0 800x600x24 +extension GLX' <legacy-probe>
```

Обязательны GL2.0/110, GL2.1/120 и llvmpipe без override как контроль среды.
Overrides меняют объявляемые версии/проверки Mesa, **не эмулируют X1650**,
не убирают все современные расширения, не воспроизводят compiler/driver bugs.
Нужен strict capability policy: запретить backend использовать функции вне
baseline, отдельно тестировать отсутствие optional extensions и заниженные
лимиты. Не считать «работает на Mesa 4.x» тестом GL2 совместимости.

Каждый запуск игры — только в свежих `mktemp -d` каталогах:
`HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`;
`HORIZON_LAUNCHER_NO_SELF_UPDATE=1`, `HORIZON_NO_AUDIO=1`.
Если потребуется launcher, также изолировать `HORIZON_LAUNCHER_TRASH`,
`HORIZON_LAUNCHER_CLIPBOARD`, `HORIZON_LAUNCHER_DESKTOP` внутри scratch home.
Звук потом проверять отдельным тестом. Данные игрока не использовать.

### Набор проверок

1. CPU unit tests: quad unpack, индексы/winding, camera-relative transforms,
   atlas pages/UV/padding, mip borders, сортировка и eviction при budget pressure.
2. GL integration tests: **выполнить** GLSL compile/link, upload, draw и readback;
   несколько страниц atlas, alpha cutout/blend, depth, clipping/scissor, lightmap,
   resize/minimize/restore и отсутствие необработанных GL errors.
3. Deterministic fixture scenes с собственными тестовыми текстурами: все базовые
   пути без Mojang resources. Сравнение изображений с tolerances/масками,
   не только exact hashes и не только «процесс завершился с кодом 0».
4. Game smoke после законной подготовки ресурсов: menus → world → edits →
   entities/items → inventory → save/reload → server. Использовать существующий
   `horizon-ctl`/control API, расширяя его только при необходимости.
5. Stress: быстрая ходьба/телепорты, многократный reload pack/мира, смена размера,
   нехватка texture budget, 30–60 минут сессии; память/handles стабилизируются.
6. Regression: прежние unit/parity harnesses и современный renderer. Графический
   порт не является поводом менять gameplay parity или удалять harnesses.

В репозиторий не добавлять Mojang assets, jars, prepared resources и большие
логи/скриншоты. CI с synthetic fixtures не зависит от лицензированных ресурсов;
полный game smoke — отдельный стенд. Отчёт содержит commit, toolchain, Mesa,
GL/GLSL strings, режим, результаты/readbacks и причины skips. Отсутствующие
зависимости/ресурсы — `blocked`/`skipped`, никогда не `passed`.

## 8. Приёмка и риски

Три разных статуса, которые нельзя смешивать:

- **Linux GL2 verified**: настоящий llvmpipe runtime, оба baseline режима,
  synthetic suite и доступный game smoke; modern backend без регрессий.
- **Win7 x86 verified**: PE32, loader/runtime/input/files/audio/TLS, ресурсы и
  gameplay проверены на самой ОС. VM допустима для ОС, но не для оценки X1650.
- **X1650 verified**: физическая плата, Win7 x86 ICD производителя, полный smoke,
  длительная сессия, память и performance report на записанных настройках.

Главные риски: Win7 API в транзитивных dependencies; Rust/std toolchain;
старый ATI GLSL compiler; VA/texture budgets; слишком много draws после
разворачивания quads; ресурсы/подготовка через современный Java toolchain.
Ресурсы разрешается подготовить на современном компьютере из своей копии игры
и перенести локально: не требовать JDK 25 на целевой Win7 x86 и не распространять
Mojang данные в portable archive. Проверить запуск без Java после подготовки.
Не обходить TLS/certificate ошибки отключением проверки сертификатов.

**Начать с этапа 0:** получить Linux GL2 runtime baseline и Win7 PE32 probe.
Следующий шаг — backend boundary и минимальный legacy-only build. До этих
проверок оценка сроков полного порта и обещание запуска на X1650 ненадёжны.


## 9. Начало реализации: проверено здесь

Добавлен независимый Rust tool [`tools/legacy-gl-probe`](../../tools/legacy-gl-probe/README.md)
и runner `tools/legacy_gl_smoke.py`. Без wgpu/игровых ресурсов создаётся desktop
GL-контекст, реально выполняются GLSL compile/link, VBO/EBO draw, clear/triangle/depth
readback, проверяется отказ для ошибочного GLSL и отсутствие GL errors.

На Linux здесь прошли **OSMesa + llvmpipe**, Mesa 23.1.4 / LLVM 17.0.6:
GL2.0 с source `#version 110`, GL2.1 с source `#version 120`, контрольный GL4.5
с source `#version 110`. JSON-отчёт: `target/legacy-gl-probe/report.json` (ignored).
2 Rust unit tests, 5 Python tests и Clippy с `-D warnings` прошли.

Обнаруженное ограничение Mesa: reported GLSL override `110` приводит к null из
`glGetString(GL_SHADING_LANGUAGE_VERSION)`. Поэтому runner использует ceiling `120`
в обоих baseline режимах; GL2.0 shader source остаётся строго GLSL110.
Это не проверка reported GLSL1.10 на настоящем старом драйвере.

EGL-код добавлен, но его runtime в этом окружении пока не прошёл из-за недостающих
библиотек. GLX, WGL/PE32/Win7 runtime и игра не проверены. Gate A остаётся
открытым. Современный renderer и terrain-прототип PR #1 не менялись.


### Продолжение: WGL bootstrap

В Rust probe добавлен Windows-путь: Win32 окно с собственным DC, выбор/проверка
pixel format, legacy `wglCreateContext`, загрузка GL2-функций с фильтрацией WGL
sentinels и fallback в `opengl32.dll`, swap после readback, thread-confined cleanup.
Общие проверки теперь диагностируют GDI Generic / GL<2 / GLES до загрузки
современных entry points. Отчёт содержит target OS, pointer bits и факт swap.

На Linux stable Rust 1.88.0 прошли 8 Rust unit tests, Clippy для host и
`i686-pc-windows-msvc`, а также Windows x86 `cargo check`. Повторно прошли все
три OSMesa/llvmpipe режима и 5 Python tests. Это **compile-only проверка Windows
ABI**, не Win7-supported build и не запуск WGL. Обычный MSVC target не подменяет
`i686-win7-windows-msvc`; PE32, imports/CRT и реальная плата всё ещё не проверены.
Добавлен отдельный CI workflow для Linux smoke и modern-Windows x86 build;
на GitHub в этой сессии он не запускался.


## 10. Готовое ядро рендера, но ещё не готовая игра

Добавлены `horizon-render-api` (CPU-контракт) и `horizon-legacy-render` (нативный
GL2 backend). Native context код перенесён из probe в библиотеку, вызовы поточно
изолированы и каждый раз привязывают нужный контекст. API покрывает RGBA8 textures,
обновления, VBO/EBO meshes, ordered draws, depth/cutout/blend/overlay, scissor,
lightmap, простой fog, readback и удаление ресурсов. Есть byte/object budgets,
проверки индексов/областей/handle IDs, f64 camera-relative subtraction и GL projection.

В CPU API переиспользована atlas packing/build-page реализация из draft PR #1;
конвертер terrain сохраняет страницы/проходы/порядок и границы u16, нормализует
материальные alpha flags. В `horizon-client-render::render_bridge` добавлен адаптер
настоящих `TerrainVertex` / `SectionMesh`, но создание renderer и игровой цикл
по-прежнему используют wgpu. API/GL библиотеки имеют отдельный dependency-free
bootstrap workspace; общий контракт подключён path dependency к game render crate.

Проверено: 12 native/API + 2 probe Rust unit tests, Clippy host и Windows x86,
13 реальных backend проверок по пикселям в каждом из 3 llvmpipe режимов.
Python regression: 633 tests, 26 skipped. Runner schema 2 требует backend checks:
старый отчёт «нарисован треугольник» больше не проходит как проверка backend.

**Блокер полной сборки:** попытка `cargo check --offline -p horizon-client-render`
остановилась на отсутствии `log` в registry cache. crates.io/static.crates.io
не разрешены сетевой политикой sandbox; ресурсов игры также нет. Root metadata
проверена, но adapter test и сборка игры не выполнены. Нужны подготовленный Cargo
cache/vendor и игровой prepared root, доступные здесь законным способом.

**Не готово:** переключение `Game`/`gfx::Renderer` на backend boundary, UI special
passes, entities/hand/CPU poses, анимации/pack reload, upload scheduler, resize/input,
mips/repeated UVs, полноценный single-player/multiplayer smoke и Win7 PE32/CRT/ICD.
Это не подмена игры синтетической сценой. Все ограничения и команды:
[`crates/horizon-legacy-render/README.md`](../../crates/horizon-legacy-render/README.md).


## 11. Продолжение: резидентные секции и GitHub vendor mirrors

Добавлен `horizon_render_api::scene::WorldScene`: загрузка atlas pages/16×16
lightmap, ограниченное число резидентных секций, поэтапная замена GPU meshes,
выгрузка и удаление ресурсов. Ошибка в середине replacement сохраняет прежнюю
секцию; временные меши удаляются перед следующей операцией. Неудавшееся удаление
не теряет ID: оно повторяется позднее. Видимость и порядок проходов задаёт caller,
включая предварительную сортировку translucent quads. Это ещё не async scheduler.
`render_bridge::upload_section` соединяет этот слой с настоящим `SectionMesh`,
проверяя общий vertex ceiling двух проходов до изменения GPU ресурсов.

Новый реальный GL test проверяет replacement rollback при превышении mesh budget,
обновление света, сохранение старого кадра при missing section, unload, Drop и
частично неудавшийся constructor. Во всех трёх OSMesa/llvmpipe режимах теперь
**14 backend checks**. Проверены host/Windows-x86 Clippy; Python suite — 641 tests,
26 skipped. Это не проверка игрового цикла или запуска Win7.

`tools/cargo_github_vendor.py` использует фиксированные Git commits vendor деревьев
ClickHouse, Firefox, ChromiumOS и rust-code-analysis. Проверяет name/version,
поле package checksum из `.cargo-checksum.json` против root `Cargo.lock`, а затем
SHA256 каждого перечисленного файла; копирует только эти файлы. Сохраняет source
provenance и отчёт под ignored `.local/`. Нормальный Cargo config и lock не меняет.
Это проверка согласованности с pinned vendor manifests, **не независимая проверка
SHA256 исходного сжатого `.crate` архива**. Chromium без package checksum и пакеты
с отсутствующими Windows import libraries не принимаются как полные копии.

Результат здесь: **287 из 488** registry packages проверены и установлены,
**201 не найден**. Неполный набор возвращает exit 2, а не успех. Повторный
`cargo --config .local/cargo-mirror/offline.toml check --offline --locked -p
horizon-client-render` прошёл прежний блокер `log`, но остановился на `cpal`.
У root render crate MSRV 1.90, поэтому установленный для bootstrap Rust 1.88
тоже не следует выдавать за готовый игровой toolchain. Полная сборка и тест
настоящего game adapter всё ещё не выполнены; prepared resources отсутствуют.

Прогресс не меняет статус приёмки: wgpu-owned `gfx::Renderer` остаётся основным,
legacy gameplay executable, меню/HUD/entities/hand и Win7 PE32/CRT/ICD не готовы.


## 12. Игровые адаптеры проверяются в собственной ветке через Actions

Создан собственный workflow `legacy-gl-branch.yml`, выполняемый для push только
в `arena/a7e96628-horizon`. Чужие PR/workflows/runs не использовались. Первый
собственный run 41772458349 прошёл independent GL2 runtime и компиляцию/тест
настоящего `render_bridge`. Второй, 41773260872, прошёл также CPU-тесты GUI bridge
и настоящий `HudAtlas::boot` + `HudBatch` -> GL2 pixel readback. Таким образом,
неполный локальный Cargo cache больше не блокирует проверку этих игровых типов:
полная dependency graph render crate собирается на GitHub.

Добавлены `Clear::Load/Depth/ColorDepth`, GUI passes с точными RGB/alpha blend
factors (invert/additive/vignette), LEQUAL+depth write для GUI моделей и linear
sampling для vignette. `gui_bridge` сохраняет GUI scale, порядок triangle-list,
alpha coverage и top-left scissor, переводит clip Z в GL -w..w. Изолированная
глубина очищается один раз на исходный run, а не на каждый u16 chunk. Глифы
обновляются через настоящие `font::Patch`; POT padding на изменяемых краях атласа
также обновляется. CPU staging имеет vertex/byte limits, неудавшийся GPU upload
освобождает временные меши до повторной попытки.

`Renderer::owned` и владеющая backend/layout `WorldScene<B>` позволяют хранить
renderer внутри Game без self-referential struct, утечки контекста или transmute
lifetime. В render crate появился opt-in `legacy-gl` и `TerrainHudRenderer`,
связывающий реальные SectionMesh/HudBatch/HudAtlas с native backend: upload/unload,
lightmap, glyph patches, HUD reload, world + GUI composition и present. Это пока
**terrain/HUD subset, не полная замена gfx::Renderer в startup**.

Локально прошли 19 standalone Rust unit tests, Clippy host/i686 MSVC и все три
OSMesa/llvmpipe режима с **15 backend checks**. Python: 641 tests, 1 skipped в
последнем запуске. Третий собственный run **41773789614 passed**: CPU game bridge tests, два
реальных GPU fixtures с HudAtlas/SectionMesh/font::Patch, owning
TerrainHudRenderer (включая HUD reload/unload) и game-bridge Clippy.

Осталось: выбрать backend в Game только после реализации entities/hand/sky,
полного GUI glint/portal/CIT, scheduling/input/resize. GUI paging добавлен в §13.
Для Win7 нужны dedicated std/toolchain и PE/import/CRT checks; modern i686 ABI
Clippy и llvmpipe не являются Win7/X1650 runtime qualification.


Четвёртый собственный run **41774182610 passed**, code commit `85623ba`:

- `cargo check --locked -p horizon-client --no-default-features --features
  horizon-client-render/legacy-gl` — проверен весь клиент, не только render crate;
- CPU game bridge tests, три настоящих GL2 pixel fixtures игровых типов,
  `horizon-client-render --features legacy-gl` lib/tests Clippy;
- independent GL2 matrix; native probe собран и слинкован для i686 MSVC на
  современной Windows, artifact `probe-modern-windows-x86-not-win7-qualified`.

Cargo check не является linking/runtime тестом игры. Feature `legacy-gl` пока
добавляет координатор рядом с wgpu, не удаляет modern dependency graph и не
переключает обычный startup. Поэтому готового legacy gameplay EXE/Win7 runtime
по-прежнему нет. Новый workflow фильтрует code paths, чтобы docs-only push в этой
ветке не запускал повторную полную компиляцию.


## 13. Большой GUI atlas: страницы, сжатый backing и ограниченный cache

Игровой `TerrainHudRenderer` больше не требует единую POT-текстуру всего HUD.
`paged_gui::Layout` делит исходное изображение на POT-страницы 512×512 с interior
510×510 и gutter в один texel. CPU clipping выполняется в f64 texture-pixel space,
интерполирует position/depth/colour/UV/light UV, сохраняет исходный порядок
треугольников и очищает depth лишь один раз на исходную группу моделей. Пределы
выходных vertices и посещаемых clipping cells ограничивают работу на плохих UV.
Сортировки по страницам нет: alpha-overlap не меняет порядок.

В игровом адаптере страницы хранятся отдельными LZ4 blocks. Профиль по умолчанию:
до 8 resident pages (8 MiB tracked RGBA payload), compressed backing до 128 MiB,
4096 virtual pages; 8 MiB для входящих glyph pixels + decoded dirty pages одного
patch batch. Реальная ёмкость cache уменьшается по доступному byte/object budget
после terrain/lightmap/white textures. Это не измерение физической VRAM драйвера.
Первоначальная сборка полного CPU atlas, reload peak и всё адресное пространство
процесса ещё требуют отдельной Win7/x86 квалификации.

Font patches сначала полностью валидируются, затем каждая затронутая страница
декодируется один раз. Обходится только dirty rectangle, а не вся страница на
каждый глиф; отдельный write-work cap защищает от повторных tiny edge patches. Обновляются также neighbouring gutters и повторяющиеся
края. Новые compressed blocks коммитятся только после проверки общего бюджета;
ошибка оставляет старый backing. LRU cache инвалидирует изменённые страницы,
вытесняет старые до новых GPU allocations и загружает их по необходимости.
Reload готовит новый backing до swap; старые GPU pages удаляются перед следующим
streamed frame. CPU GUI conversion и проверка transient mesh capacity проходят
до очистки world framebuffer. При driver/upload failure посреди streaming
полного GPU rollback нет — частично нарисованный кадр возможен.

Оба собственных Actions runs **41776018494** и **41776564666** прошли все три jobs.
Последний проверяет code commit `d5dbd2b`:

- полный `horizon-client` Cargo check с `legacy-gl`, CPU bridges/backing и game Clippy;
- **пять actual-game GL2 fixtures**, включая синтетический HudAtlas 16385×2:
  при запросе восьми страниц GPU budget позволяет только одну 512×512; pixel
  readbacks подтверждают seams и font patch через границу. Отдельный fixture
  подтверждает сохранение предыдущего кадра при нехватке transient mesh budget;
- independent OSMesa/llvmpipe GL2.0, GL2.1 и native: **16 backend checks на режим**.
  Проверяются alpha-order, линейные gutters, глубина при eviction, dirty reload
  и полное освобождение ресурсов;
- ordinary i686 MSVC probe вновь собран/слинкован на современной Windows.

Локально: 22 standalone Rust tests, host/i686 Clippy, 3×16 Mesa checks и Python
641 tests (1 skipped). Это всё ещё не переключает Game startup с wgpu: entities,
hand, sky, специальные pack shaders, input/resize и dedicated Win7 toolchain /
PE/import/CRT/ICD/runtime qualification остаются незавершёнными. Готового
playable Win7 EXE этот этап не предоставляет.

После ограничения glyph patch work прошёл также собственный run **41777293951**
(code `0aa4a05`, все три jobs). Новые CPU tests перебирают все 90 rectangles
источника 5×3 и сравнивают patched pages с полным rebuild; повторные tiny edge
writes сверх work budget отклоняются без изменения backing. Полный client check,
game Clippy, пять actual-game GL2 fixtures, independent matrix и Windows x86
probe linking повторно прошли. Это последний проверенный code commit этапа.


## 14. Общий мировой проход entities/items/particles и первый план руки

Добавлен `entity_bridge` на настоящих `mesher::Vertex`, без wgpu/GL handles на
границе CPU. API `entity::runs` сохраняет coverage alpha, декодирует SNORM normals,
считает cardinal lighting (включая Nether и отдельно front/back) и bilinear
lightmap per vertex. Vertex texture fetch не нужен: GL2 не гарантирует его наличие.
Hurt/TNT/white overlay смешивается в GLSL после texture×tint и перед lightmap,
как в существующем `entity.wgsl`, а не запекается неверным RGB multiplier.

Entity mesh имеет проверенный stride 76 bytes: обычный Vertex + back colour +
light colour. Terrain/GUI сохраняют stride 44. Native backend проверяет совпадение
typed mesh/pass до clear; использует шесть attributes, GLSL110, back-face culling,
LEQUAL и alpha blend. Bodies и translucent items пишут depth; translucent particles
не пишут. CPU clipper интерполирует дополнительные атрибуты и учитывает 3D area:
мировая плоскость с постоянным X больше не исчезает при повороте камеры.

`TerrainHudRenderer::render_geometry` выполняет последовательность:

1. solid/cutout terrain;
2. обычные entity/item bodies;
3. translucent terrain;
4. translucent particles;
5. hand — собственная GL projection, clear только depth, без world fog;
6. HUD.

Все textured passes используют один ограниченный atlas LRU. Порядок внутри фаз
не меняется. Entity/hand inputs уже camera-relative и CPU-posed; f64 camera не
прибавляется к их f32 positions. Общие vertex ceilings и transient mesh payload
проверяются до очистки мира. Retired transient meshes сохраняют ID для cleanup
retry. GPU/driver failure посреди streaming всё ещё может оставить частичный кадр.

Поддержаны overlay codes 0, 1, 5, 15, 16..31, 32. Glint, portals, shadows,
crumbling, energy swirl, dissolve и неизвестные flags пока отклоняются явно.
Это не полный entity material coverage и не адаптер всего `gfx::Frame`:
visibility/sorting, skin-frame CPU fallback, дополнительные passes, sky,
startup/input/resize и Win7-specific std/PE/import/CRT/ICD/runtime остаются работой.
Проекция caller должна иметь обычный GL depth, не reversed-Z современного gfx.

Собственный run **41779419733** (code `7dd78a8`) прошёл все три jobs: полный client
Cargo check, game CPU bridges/Clippy, **шесть actual-game GL2 fixtures**, independent
matrix и modern Windows i686 probe build/link. Новый combined fixture проводит
terrain/entities/particles/hand/HUD через одну resident page 4×4, проверяет
alpha/depth порядок, lightmap update, unsupported-material failure до clear,
сохранение кадра при нехватке transient budget и освобождение временных meshes.
Independent readbacks проверяют обе стороны normals/culling, hurt overlay order,
интерполяцию light через seams, 3D clipping и typed-buffer rejection.

Локально прошли **24 standalone Rust tests**, host/i686 Clippy, **3×17 Mesa checks**
и **642 Python tests (1 skipped)**. Первый run этапа 41779016236 имел failure в
independent job; положительный Python report fixture не включал новый backend
outcome. Fixture исправлен, добавлен тест обязательности каждого outcome, и
финальный run полностью зелёный. Обычный Game startup всё ещё использует wgpu;
готовый playable Win7 EXE и FPS на X1650 не заявляются.


## 15. Тени, water-mask лодки и мировые текстовые слои

Добавлены material kinds для biased text (35), shadow (14), water mask (13) и
отдельного `see_through` stream. Classic submission теперь соответствует
существующему игровому `gfx`: bodies → biased text → shadows → translucent items
→ boat masks → translucent terrain → particles → see-through text → hand → HUD.
Внутри kind сохраняется входной порядок; сортировки по atlas page нет.

Тени не используют entity cutout 0.1, поэтому слабая тень не исчезает; cull back,
без depth writes, с camera-facing polygon offset. Text 35 использует cull back,
собственный depth bias и depth writes. Bias bodies/text/shadows переведён со знака
reversed-Z на обычный GL depth через glPolygonOffset; точное поведение на ATI и
разных depth formats ещё требует проверки. Water mask пишет только depth: shader
возвращает результат до texture/alpha sampling, RGBA writes отключены. Неиспользуемые
UV канонизируются, GPU atlas page для mask не загружается. Color mask, polygon
offset, culling и vertex layouts сбрасываются как внутри draw list, так и между
views; clear восстанавливает RGBA writes после depth-only кадра.

`Geometry::see_through` — настоящий мировой текстовый проход после particles,
до hand. Он сохраняет world projection/light/fog, но не тестирует и не пишет
depth; это не GUI overlay. Back faces отсеиваются. World-only materials в hand
отклоняются до clear; hand item-sheet quads используют обычный hand pipeline bias.

Input и clipped output всех entity/hand/name-tag streams делят один vertex budget.
Каждый stream получает оставшийся лимит до allocation следующего. Временный
unpaged entity/GUI scratch освобождается до GPU submission, а не удерживается
shadowed bindings до конца функции. Driver/upload failure посреди streaming всё
ещё не гарантирует framebuffer rollback.

Собственные runs **41781489098** (`4679b61`) и **41782002701** (`d648057`) прошли
все три jobs: полный client check, CPU bridges/game Clippy, **семь actual-game GPU
fixtures**, independent matrix и ordinary Windows x86 probe build/link. Новый
fixture намеренно подаёт mask/item/shadow в обратном порядке и проверяет faint
shadow под item, water exclusion на половине кадра, text bias/culling, name tags
за terrain, hand composition и expanded vertex budget до clear. Independent
fixture дополнительно проверяет RGBA/depth/bias/layout transitions в одном
смешанном draw list и world fog для see-through текста.

Локально: **26 standalone Rust tests**, host/i686 Clippy, **3×18 Mesa checks**,
**642 Python tests (1 skipped)**. GUI scratch-lifetime cleanup и исключение
Markdown из code-path workflow filters добавлены следом; финальная проверка
фиксируется ниже. Startup остаётся wgpu. Sky, glint/portal/crumbling/energy/dissolve,
остальные gameplay passes, input/resize и Win7 toolchain/PE/CRT/ICD/runtime ещё
не завершены — готовый playable Win7 EXE не заявляется.


Финальный собственный run **41782569337** (code `5fa39c0`) прошёл все три jobs,
включая GUI scratch cleanup, семь игровых GPU fixtures, полный client check и
Windows x86 probe linking. Это последний проверенный code commit этапа. Negative
Markdown path pattern исправляет прежние crate-wide globs: README-only push
больше не запускает повторную полную сборку.


## 16. Glint: entity/item/armor/hand, без обхода лимитов

Добавлены materials 7/8/33/36 при явно установленных item/armor images. Вершина
actual `mesher::Vertex` передаёт независимый `glint_uv`; clipping интерполирует его
вместе с light/back colour, не переупаковывая как atlas UV и не выполняя `fract`
на вершинах. Новый checked stride — **84 bytes**, семь attributes, три samplers:
GLSL110/GL2 minima достаточны. Armor использует affine scale .16, rotation 10°,
110000/30000 tick offsets; общий период 330000, отдельная armor image.

Порядок соответствует существующему `entity.wgsl`: texture*tint → alpha cutout →
overlay → light → `(strength*foil.rgb)^2` → max(alpha,strength) → fog. Alpha foil
не участвует; accessibility strength — точный finite f32 [0,1]. Sampler repeat +
linear, состояние nearest/clamp восстанавливается для обычного atlas draw.
Nonconstant glint нельзя совмещать с atlas/lightmap texture ID.

Coordinator получает `Geometry::glint` и staged `set_glint_textures`: обе images
проверяются/выделяются до swap, old assets сохраняются при ошибке/нулевой квоте
страниц. Retired images удаляются до следующего preflight/page upload. Восьмой
actual-game GPU fixture проверяет world/hand, оба rollback пути, повторные reloads,
alpha и точный budget **1164 bytes textures / 516 bytes transient mesh**.

Локально выполнены **27 Rust unit tests**, host Clippy и **3×19 OSMesa llvmpipe
outcomes**. Новый outcome проверяет repeat/linear, 60 sample positions через швы
страниц, armor clock/image, half/zero strength, hurt/light/fog, cutout/alpha,
недопустимый alias, reset sampler state и полное освобождение ресурсов.
Собственный [run 41785733646](https://github.com/lukintrees/horizon/actions/runs/41785733646)
на `1ba6133` прошёл все три jobs: **восемь actual-game GPU fixtures**, full-client
check/game Clippy, independent runtime matrix и modern Windows x86 probe link.
Полный локальный Python regression: **642 tests, 1 skipped**.

Это не готовая Win7 игра: startup/frame/resource adapter остаётся wgpu, GUI/CIT
блеск и portal/crumbling/energy/dissolve ещё не реализованы, sky/input/resize и
реальный Win7/x86/X1650 runtime не квалифицированы.


## 17. Vanilla GUI glint: инвентарь, маски и depth-группы

Добавлен путь `gui_bridge::glint_runs` из настоящего `HudBatch/HudVertex`:
marker `uv.x >= 2` отделяется до atlas clipping, а независимый `glint_uv`
интерполируется без перепривязки к странице. GUI имеет собственный компактный
layout **52 bytes**, не 84-byte entity lighting. Backend проверяет Basic/Entity/Gui
mesh/pass соответствие до clear; обычный старый GUI API остаётся совместимым.

Шейдер следует существующему `hud.wgsl::fs_main`: item alpha mask, discard <.1,
RGB `(HudBatch::glint_alpha * foil.rgb)^2`, alpha = mask. Tint, world lightmap и
foil alpha не участвуют. Additive использует ONE/ONE, поэтому half mask не
умножает RGB; alpha/depth pipeline по-прежнему использует source-alpha blending.
GUI strength независима от world strength, та же managed item image обслуживает
обе системы. CIT/portal остаются явными ошибками.

Один clear на исходную model depth-группу сохраняется при plain/foil/u16/page
split. Исправлен отдельный крайний случай: entirely degenerate prefix больше не
теряет pending depth clear при смене материала; clear не протекает в следующий
обычный overlay. GPU проверяет оба порядка depth-материалов, новую model group,
scissor и восстановление состояния после glint.

Локально: **31 standalone Rust tests**, host Clippy, **3×20 Mesa outcomes**,
**642 Python tests (1 skipped)**. Новый обязательный `backend_gui_glint`
сравнивает 60 repeat/linear samples через три страницы при resident cache=1,
проверяет mask/cutout/half/zero strength, reset, alias/layout rejection до clear
и cleanup. Девятый actual-game GPU fixture охватывает dynamic patches, atlas +
foil reload, сохранение кадра при неверных данных/лимитах и точный budget
**1164 bytes texture / 324 bytes mesh**. Собственный
[run 41791059747](https://github.com/lukintrees/horizon/actions/runs/41791059747)
на `6c87c53` прошёл все три jobs: девять actual-game GPU fixtures, full-client
check/game Clippy, runtime matrix и modern Windows x86 probe linking.

Это ещё не запуск полноценной игры: startup/frame adapter и scheduling/input/
resize остаются незавершёнными, Win7/PE32/CRT/ISA/ICD и реальный X1650 не проверены.


## 18. Жизненный цикл поверхности и native WGL input

Добавлены `Context/TerrainHudRenderer::{resize,poll_events,set_title,present}`.
Resize не пересоздаёт GL context: OSMesa заменяет owned backing после glFinish,
EGL staging меняет pbuffer (retired surface ограничен одной неудачной очисткой),
WGL меняет client rect существующего HWND/DC/context. Размер 1..4096 проверяется
до native изменений; при minimization WGL публикует [0,0], render/present
возвращают suspended error. После успешной смены размера содержимое framebuffer
не обещается: caller перестраивает projection/HudBatch и перерисовывает весь кадр.
Native framebuffer/pbuffer memory не входит в texture/mesh payload accounting;
resize может временно удерживать старую и staged поверхность.

WGL callback держит стабильный Box<State>, private pointer очищается до Drop
окна. Class name уникален для нескольких окон одного процесса. WM_CLOSE/Alt+F4
выдают persistent close request, не уничтожая drawable за спиной GL renderer.
Очередь/один poll ограничены 256 events/messages; adjacent motion/resize
coalescing не пересекает key/button boundaries. Overflow явно сообщается,
caller обязан сбросить held input на loss/focus/capture loss. Есть VK/scan/
extended/repeat-count, корректный UTF-16 Unicode (включая surrogate pair), signed
pointer coords, пять кнопок, horizontal/vertical wheel и mouse capture/loss.
HWND filtering сохраняет queued messages других окон, WM_QUIT возвращается outer
loop. Native modal moving/sizing всё ещё может блокировать DispatchMessage;
count bound не является hard latency bound. Raw/relative mouse и fullscreen/DPI
policy пока не завершены.

`backend_surface_lifecycle` проверяет повторные 96x48/31x79/1x1/128x32/64x64,
неизменные ID/counters, update/sample сохранённой texture, новые readback bounds,
сохранность кадра при invalid/same-size resize и cleanup. Десятый actual-game GPU
fixture сохраняет resident terrain/pages/foil, перестраивает HudBatch и проверяет
composite после пяти размеров при точном budget **1164 texture / 512 mesh bytes**.

Локально: **36 standalone Rust tests**, host/i686 Clippy, **3×21 OSMesa checks**.
Собственный Windows job теперь не только компилирует/линкует: он запускает CPU
unit tests и настоящий hidden-window test с двумя HWND без GPU ICD — callback ABI,
resize/title, posted keyboard/Unicode, queue isolation и close-before-Drop.
Собственный [run 41795980600](https://github.com/lukintrees/horizon/actions/runs/41795980600)
на `3339003` прошёл все три jobs: **десять actual-game GPU fixtures**, full-client
check/game Clippy, independent runtime matrix, **35 реально выполненных x86
Windows workspace tests** и probe linking. Полный Python: **642 tests, 1 skipped**.

Native x86 execution обнаружил ошибочный SWP_NOSIZE и caption minimums, которые
compile-only ABI check не видел. Исправлены flags; WM_WINDOWPOSCHANGING default
constraint обходится только на время validated programmatic resize, включая
initial sizing. Interactive tracking сохраняет системные пределы. Win32 test
теперь доказывает точные 1x1/96x48, Unicode/queue isolation и HWND teardown после
Drop. GPU unload assertion выполняется после documented scene garbage collection.
Предшествующие failed runs не считаются подтверждением; финальный run включает
все исправления. Диагностика CPU/GPU failures публикуется в own-branch check
annotations: sandbox не может скачать redirected Actions log blobs.
EGL resize только compile-qualified здесь; настоящий WGL GPU, Win7/X1650,
PE/CRT/ISA, startup/frame/input wiring и оставшиеся sky/material passes ещё
не квалифицированы как полная playable игра.


## 17. Подготовка настоящего gfx::Frame

Добавлены `legacy_gfx::frame::Prepared::new` и
`TerrainHudRenderer::render_game_frame`. Принимают настоящий игровой Frame,
размер drawable, authoritative visible lists и brightness/fullbright options.
World projection/view/bob/spin и отдельная hand projection сохраняют существующую
арифметику игры. `gl_reversed_projection` делает z'=w−2z: reversed near=1/far=0
переходят в обычные GL near=−1/far=+1 с LESS и clear-depth=1. Старый
`gl_projection` для НЕ-reversed матриц здесь применять нельзя.

CPU lightmap вынесена без изменения формулы в общий `lightmap::pixels`, выбор
fog uniforms — в `fog::world_parameters`; их вызывают оба backend. GL2 fog теперь
поддерживает max(environment spherical, render-distance cylindrical) × color
alpha. Cylindrical distance = max(length(xz), abs(y)); расстояния интерполируются
так же, как WGSL. Равные диапазоны являются step/disabled MAX-парой с тем же
порядком ветвей, а не делением 0/0. Старый spherical-only контракт сохраняется
через `Fog::game=None`. Не заявляется полная parity всех специальных материалов
и hand/screen/sky fog.

Видимые static block-entity snapshots переводятся из section-local в
camera-relative с вычитанием в f64; порядок visible list сохраняется перед
обычными entities. Невидимые модели не копируются; без static геометрии source
заимствуется. Действует общий limit static+dynamic+hand+see-through, дубликаты
visible keys/snapshots отвергаются, scratch allocation fallible. Resident static
GPU cache, streaming visibility/upload scheduling здесь не добавлены.

До submission проверяются камера/матрицы, HUD size после resize, fog/lightmap,
glint, geometry counts/limits и неподдержанные requested passes. Panorama, sky,
clouds/celestial/stars, lines/outlines, GPU skins,
glowing, dynamic/colored lights, emissive layers, screen overlays, border,
post/menu blur, chunk fade и water mirror возвращают явную ошибку — не тихо
отключаются. Поэтому это пока subset без видимого sky и перечисленных эффектов,
не готовый обычный Overworld/title frame. Стандартный Game всё ещё wgpu.
Lightmap/resources не образуют транзакцию; CPU rejection сохраняет framebuffer,
а GPU failure может оставить частичный кадр.

Добавлены четыре CPU frame tests, один exact RGBA8 lightmap test и 11-й
actual-game GPU fixture: настоящая perspective при camera x=30,000,000.25,
resident terrain, static/dynamic occlusion, общий lightmap, fog override/alpha,
hand после depth clear, HUD и полное сравнение framebuffer после invalid frames.
Обязательный independent `backend_game_frame_inputs` проверяет dual fog и
unreversed depth без игрового dependency graph. Локально **37 standalone Rust
tests**, host/i686 Clippy, **3×22 OSMesa checks**, **642 Python, 1 skipped**.
Win7/X1650, PE/CRT/ISA, startup/platform/game-loop/resource wiring и оставшиеся
проходы ещё не дают основания называть порт playable.

Финальное подтверждение своего branch-only CI:
[41799081051](https://github.com/lukintrees/horizon/actions/runs/41799081051)
на `e568d92`: **все три jobs SUCCESS**, четыре frame + один lightmap CPU tests,
**11 actual-game GPU fixtures**, full-client check, game Clippy, independent
runtime matrix, **36 выполненных x86 Windows workspace tests** и probe linking.
Следующий docs-only commit не меняет проверенный код. Ошибки CPU/Clippy теперь
также попадают в own check annotations без скачивания redirected log blobs.


## 18. Экранные эффекты настоящего игрового кадра

`Frame.screen` больше не отвергается целиком: alpha/vignette quads из
`screen_effects` рисуются после world/particles/see-through/hand и до HUD, в
исходном порядке. Это SDR-вариант воды, огня/block overlay, vignette,
pumpkin/powder-snow/portal; `world_tint` имеет отдельный смысл только в HDR,
который legacy profile не включает. Остальные missing passes остаются errors.

Спрайты извлекаются из compressed virtual HUD backing: максимум одна decoded
page одновременно, копируются interiors без gutters, giant atlas не собирается.
Одинаковые rect делят CPU pixels в пределах кадра; patches применяются до
extraction, reload не оставляет старый sprite cache. Rect должен соответствовать
целым texel boundaries, восстанавливаемым из atlas f32 (допуск .01 texel),
subtexel crops отвергаются. До clear проверяются vertices/flags/native texture
size, **256 quads / 8 MiB extracted pixels + одна decoded page** и GPU budgets.

`Pass::Screen` получает отдельную texture со sprite-local UV, без CPU fract.
Nearest/linear, repeat/clamp применяются hardware после интерполяции, включая
bilinear на шве спрайта. Разрешены NPOT textures desktop GL2 core без mipmaps и
без resample/padding; terrain/GUI pages остаются POT. В runtime probe теперь
обязателен реальный 3x2 NPOT repeat/linear readback, но это не ATI qualification.
Shader обходит light/fog/glint/GUI cutoff. Vignette игнорирует alpha для RGB,
сохраняет destination alpha; ordinary draws сбрасывают screen/sampler state.

CPU preflight резервирует max(largest sprite, GUI page), а не их сумму: cache
освобождается до world sizing и после hand, screen держит только одну image +
188-byte mesh, удаляет их до HUD. Failed cleanup IDs не теряются. Native failure
по-прежнему может оставить partial frame; CPU rejection сохраняет framebuffer.

Новый обязательный outcome `backend_screen_sampling`: 160 независимых RGBA
samples для 8 blend/sampler combinations, transparent vignette, light/fog ignore,
alias/reset, tiny alpha и cleanup. Два CPU tests перебирают все 420 subrectangles
7x5 atlas, seams, dedup, patch и invalid budgets. 12-й actual-game GPU fixture
проверяет Frame.screen, hand→screen→HUD, четыре sampling режима, patches/reload,
rejection preservation и **1156 texture / 516 peak mesh bytes**.
Локально: **38 standalone Rust tests**, host/i686 Clippy, **3×23 OSMesa checks**,
**642 Python, 1 skipped**. Startup/input/resources, sky/panorama/оставшиеся passes,
Win7 PE/CRT/ISA и настоящий X1650 пока не дают playable qualification.

Финальный own CI [41801421945](https://github.com/lukintrees/horizon/actions/runs/41801421945)
на `c076df5`: **все три jobs SUCCESS**, два новых CPU tests, **12 actual-game GPU
fixtures**, full-client check/game Clippy, 3×23 independent runtime outcomes,
**37 выполненных x86 Windows workspace tests** и ordinary probe linking.
Первый GPU запуск корректно отверг недостаточный hand-mesh budget самого теста;
финальный fixture учитывает шесть paged vertices. Docs-only commit после него
не меняет проверенный код. Чужие workflows/PR/Actions не использовались.


## Реальные world lightning и additive/rays (2026-11-04)

`render_game_frame` теперь обрабатывает настоящий `Frame.lightning` и
`Frame.additive`, а не только отдельный тестовый draw. Молнии используют общий,
неизменённый seed-generator игры: **896 vertices / 224 quads на bolt**. Сначала
в f64 вычисляется bolt−camera, затем f32 offset прибавляется к local f32
geometry. Порядок входных bolts сохраняется, после них идёт source additive.
Постоянного legacy seed-cache пока нет: временно строится один bounded template.
По умолчанию максимум **64 bolts**; generated + additive входят в единый лимит
static/dynamic/hand/see-through vertices. Overflow, nonfinite translations,
неподдерживаемый material и CPU/GPU budgets не приводят к тихому пропуску.

Отдельный `entity::additive_runs` принимает только коды **0/2/3/37**; обычный
body/hand путь по-прежнему не разрешает additive/rays. 0/3 сохраняют CPU lightmap,
2 — unlit; overlay/glint/cutout/back-color correction и world fog здесь не
применяются. Dragon death rays (37) не используют texture/lightmap/UV и гасят
**RGB и alpha** по максимуму spherical/cylindrical fog ramps, без множителя
fog-color alpha. Обе политики: **SRC_ALPHA/ONE для RGB и alpha, LEQUAL**.
Обычный additive — без cull/depth-write, rays — back-cull с depth-write даже при
alpha=0 или полном fog fade. Shader mode сбрасывается на каждом draw.

Порядок coordinator: solid → body/text/shadow/item/water-mask → lightning +
additive → translucent terrain → particles → see-through → hand → screen → HUD.
Paging, mesh/page budgets и pre-clear validation распространяются на новый pass.
24-й independent GPU outcome проверяет blend/depth/cull/fog/material/state-reset.
13-й actual-game GPU fixture проверяет реальные seeded bolts возле 30 млн блоков,
две instances одного seed, полное fog, покрытие молнии translucent terrain,
ray depth и сохранение framebuffer при rejection. Coverage terrain в этом тесте
задаётся texture alpha: старший байт mesher color — material/HDR flags, не alpha.

Локально: **40 standalone Rust tests**, host/i686 Clippy,
**3×24 OSMesa checks**, **642 Python tests, 1 skipped**. Полноценный startup,
input/resource backend, sky/End/panorama/stars/clouds, weather и прочие отклоняемые
passes остаются незавершёнными. Windows x86 ABI/linking не заменяет проверку
PE/CRT/ISA/ICD на Win7 32-bit и настоящем Radeon X1650; playable qualification
по-прежнему не заявляется.

Финальный own CI [41805814823](https://github.com/lukintrees/horizon/actions/runs/41805814823) на `083bb8e`: **все три jobs SUCCESS**.
Full-client check, game Clippy, новый CPU test, **13 actual-game GPU fixtures**,
**3×24 independent runtime outcomes**, **39 выполненных Windows x86 workspace
tests**, ordinary x86 probe linking. Первый запуск выявил ошибку GPU-фикстуры
(coverage в mesher flags вместо texture alpha); второй прошёл GPU, но выявил
требования нового Clippy к тестовому коду. Финальный запуск включает исправления.
Следующий docs-only commit не меняет проверенный код; чужие Actions/PR не читались.


## Дождь и снег в настоящем Frame (2026-11-04)

Legacy adapter теперь принимает настоящие `weather::Vertex` и masked
`weather::Time`. Без CPU tiling: rain сохраняет порядок f32 операций и signed
remainder, snow восстанавливает исходные f64 bases и округляет base+f32 drift
как общий CPU reference. Phase прибавляется к UV до interpolation, но wrapping
выполняется только в fragment shader. Новых требований сверх GL2/GLSL 1.10 нет.

Rect weather — origin+extent, не origin+end screen. После patches нужные sprites
извлекаются из compressed backing, включая seams страниц. NEAREST/CLAMP +
shader fract повторяют только sprite, не atlas и не gutters, в том числе NPOT.
Одинаковый sprite остаётся на GPU между соседними columns; смена sprite не
меняет source order. Одновременно живут один image и один **348-byte lit mesh**.
Общий transient owner сохраняет handles при cleanup error; перед text/hand/HUD
погодные ресурсы освобождаются, GUI pages загружаются заново по необходимости.

Погодная политика: no cull, LEQUAL, no depth-write; RGB SRC_ALPHA/ONE_MINUS_SRC_ALPHA,
alpha ONE/ONE_MINUS_SRC_ALPHA. Cutout 0.1 выполняется до lightmap; используются
общие bilinear RGBA lightmap и world fog с fog-color alpha, без back-color
correction. Порядок: translucent → particles → weather → see-through → hand →
screen → HUD. Default limits: **4096 quads**, **8 MiB extracted+decoded bytes**,
плюс общий frame vertex budget, с резервом под weather до paged expansion других
streams. Ошибочные clocks/metadata/float/crop и budgets не очищают прошлый кадр.

25-й обязательный independent GPU outcome проверяет sampling/blend/cutout/light/
fog/depth/cull/reset/cleanup. Три CPU tests сверяют rain/snow с прежним независимым
column reference, negative/large ticks и double rounding, seams/dedup/patches и
rejections. 14-й actual-game GPU fixture проверяет animation, repeated/switched
sprites, порядок transparent/text/hand/screen/HUD, patches/reload и framebuffer
preservation. Exact fixture: **1156 texture bytes**, 188 resident mesh bytes +
348 transient; budget 535 отвергается до clear. Локально: **40 Rust**, host/i686
Clippy, **3×25 OSMesa**, **642 Python, 1 skipped**.

Это не завершённый playable backend: startup/input/resource wiring, sky/End/
panorama/celestial/stars/clouds и другие отклоняемые passes остаются. Проверки
Windows x86 ABI/linking не заменяют запуск на Win7 32-bit / настоящем X1650.

Финальный own CI [41808986326](https://github.com/lukintrees/horizon/actions/runs/41808986326) на `866c613`: **все три jobs SUCCESS** —
три новых погодных CPU tests, **14 actual-game GPU fixtures**, full-client check
и game Clippy, **3×25 independent readbacks**, **39 выполненных Windows x86
workspace tests**, ordinary probe linking. Первый полный погодный commit также
прошёл own CI 41808013392; финальный дополнительно проверяет reuse images и
repeated/switched sprites. Docs-only follow-up не меняет проверенный код.
Чужие ветки, PR, workflows и Actions не читались.
