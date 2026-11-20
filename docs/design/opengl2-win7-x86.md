# План: OpenGL 2.0/2.1, Radeon X1650, Windows 7 x86

> **Актуальный план запуска и параллельных PR:** [legacy-gl-launch-plan.md](legacy-gl-launch-plan.md).
> Ниже сохранены исходный дизайн и последовательный журнал реализации. Старые статусы/таблицы
> относятся к своим этапам, а не к текущему покрытию. На `d760a9a` уже проверены native GL2,
> terrain/HUD/entities, weather и классическое небо через настоящий Frame; обычный Game startup
> и Win7/X1650 runtime пока не завершены.

> **Срез 2026-11-17: работа перенесена на `upstream/main`.** Ветка `legacy-gl` (база
> `5c9c56be8c`) держит `horizon-render-api` и `horizon-legacy-render` в workspace,
> CPU-контракт кадра в `horizon-client-render` (`lightmap`, `render_bridge`,
> `entity_bridge`, `gui_bridge`, `paged_hud`) и e2e: probe `tools/legacy-gl-probe`
> через `tools/legacy_gl_smoke.py` на llvmpipe (GL 2.0/2.1, GLSL 110/120).
> Интеграция в `Game` (`RendererOwner`, окно, `--renderer native-gl2`) — следующая
> партия. `arena/…` больше не интеграционная база; этапы и таблицы ниже — история
> форка, а не текущее покрытие.

Исходный статус документа: **план и ранние прототипы**. Дата: 2026-11-04.
Историческая база первоначального аудита: `8dbbde43501924d16497d251e6e9d08f812eafcd`.
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


## Классическое небо целиком в Frame (2026-11-04)

Добавлены atmosphere, sunrise/sunset, source celestial (sun/moon/flash), stars,
End sky и нижний dark disc. Учитываются `sky_blocked` и внешний skybox/end_sky gate.
Clouds и panorama остаются отдельными неготовыми passes, не скрываются автоматически.

Восьмисекторные fans y=16/-4, radius=512 воспроизводят аналитический WGSL disc,
а не подменяют его градиентом: perspective interpolation вершинных distances
даёт тот же radial blend до сферического rim и цилиндра. Fog: (0,sky_end),
(sky_end,sky_end), с fog-color alpha. Flatten clip z сохраняет XY/w и не даёт
world near/far обрезать analytic discs; на следующем draw этот mode сбрасывается.
Sunrise использует общий 16-triangle generator. Base/sunrise/dark disc — без
bob/spin, celestial/stars — с world pose и star rotation, как исходный renderer.

End переиспользует tiled generator на 6144 vertices и tint 0x282828. Celestial
0/2/3 идут через общий unlit/CPU-lightmap additive+paging conversion, но без depth
TEST и WRITE. Stars используют исходный seed 10842, bounded immutable CPU cache,
finite/clamped brightness и SDR brightness². GPU star-cache и HDR alpha marks
не выдаются за реализованные.

Порядок: clear → base/sunrise → End/celestial → stars → dark disc → SolidLoad
terrain → прежние entity/transparency/weather/hand/HUD. WorldScene до clear
проверяет все visible keys и relative origins; sky meshes участвуют в preflight.
Default sky limit **32768 vertices**, включая basic и paged celestial expansion;
source celestial >8192 — error, не silent truncation. Basic mesh ≤9000 vertices,
индексы на bounded stack; один native basic mesh одновременно. Невалидные End
sprites/colors/rotations/fog/geometry и budgets не стирают прошлый кадр.

26-й independent readback проверяет depth/flatten/fog/blend/reset/SolidLoad.
Два CPU tests проверяют порядок, gating, shared geometry, pose/fog и rejection.
15-й actual-game GPU fixture сверяет 25 pixels с независимым analytic ray/octagon
oracle WGSL возле 30 млн блоков; затем sunrise, celestial/foreground terrain,
синтетический белый End, dark disc, stars/rotation/brightness² на 512×512,
missing keys/budgets и границу **2208 mesh bytes** для sunrise.
Локально: **40 Rust**, host/i686 Clippy, **3×26 OSMesa**, **642 Python, 1 skipped**.

Startup/input/resource wiring, clouds/panorama/lines и прочие отклоняемые passes
ещё не завершены; playable Win7 32-bit / X1650 qualification пока нет.

Финальный own CI [41811072027](https://github.com/lukintrees/horizon/actions/runs/41811072027) на `d760a9a`: **все три jobs SUCCESS**.
Два новых CPU sky tests, **15 actual-game GPU fixtures**, full-client check/game
Clippy, **3×26 independent readbacks**, **39 выполненных Windows x86 workspace
tests**, ordinary probe linking. Первый запуск выявил две устаревшие предпосылки
фикстур: sky теперь поддерживается, а full-fog terrain не обязан оставаться red.
Финальная проверка использует panorama rejection и отделяет depth от fog; End и
stars также пройдены. Docs-only commit не меняет код; чужие Actions/PR не читались.


## 19. Сборка и запуск backend'а для Windows 7 с Linux (измерено 2026-11-18)

Это операционная памятка: как собрать legacy backend под Win7 с Linux-машины и что
из этого сегодня действительно проверено. Числа и строки ниже сняты на этой машине:
база — `mvp2-win7-docs` (`318214da68b`), сборка игры и её PE — коммиты
`dc2fd86fe4c`/`92c2990ec87` ветки `mvp2-win7-build`, WGL/Wine — коммит
`34baca46702` ветки `mvp2-wine-wgl` и повторный прогон их харнесса здесь. Ни
одна строка не взята из планов выше.

### 19.1. Зачем вообще Win7

Современный renderer — wgpu 30.0.1. На Windows он собирается только с `dx12`
(`crates/horizon-client-render/Cargo.toml:79`), а GL-путь wgpu требует desktop
OpenGL 3.3 (`third_party/wgpu-hal/src/gles/adapter.rs:450`: «we minimum-version to
GL 3.3»). Машина класса Radeon X1650 на Win7 SP1 x86 не даёт ни D3D12, ни GL 3.3:
её драйвер — desktop GL 2.0/2.1. Поэтому такой машине нужен отдельный backend
`horizon-legacy-render` (GLSL 1.10, GL2 entry points), а не понижение настроек
современного.

### 19.2. Почему не хватает stable Rust

Win7-таргеты есть в `rustc --print target-list` и у stable, и у nightly, но std для
них не распространяется: это Tier 3. На этой машине `rustup target add --toolchain
nightly i686-win7-windows-gnu` отвечает:
`toolchain 'nightly-x86_64-unknown-linux-gnu' has no prebuilt artifacts available`.
`cargo check --target x86_64-win7-windows-gnu` на одной из legacy-крейтов падает с
`E0463: can't find crate for std`. Значит std надо собрать самому: nightly +
компонент `rust-src` + `-Zbuild-std=std,panic_abort`.

Проверено на `rustc 1.102.0-nightly (59cbdee2e 2026-11-17)`: оба GNU-таргета
собираются с `-Zbuild-std`. MSVC-таргеты (`*-win7-windows-msvc`) на Linux только
пишутся в кэш `check`; линковать их здесь нечем.

### 19.3. Команды: `tools/win7_build.py`

C-линкер — mingw-w64 (`mingw-w64-gcc` на Arch, `gcc-mingw-w64-*` на Debian).
Пакеты, которые нужны на этой машине, уже стоят: `i686-w64-mingw32-gcc`,
`x86_64-w64-mingw32-gcc`, `objdump`.

Рецепт живёт в `tools/win7_build.py` (ветка `mvp2-win7-build`, в `mvp2` пока
нет). Это и есть front end; ниже — то, что он делает.

```sh
python3 tools/win7_build.py                                   # оба таргета, крейты backend'а
python3 tools/win7_build.py --target i686                     # только 32-бит, форма X1650
python3 tools/win7_build.py --profile fast -p horizon-game \
  --features legacy-gl                                        # вся игра, играбельный профиль
python3 tools/win7_build.py --check -p horizon-game --features legacy-gl
```

Что он делает, по коду: выбирает rustup-toolchain (`--toolchain`, иначе
`$RUSTUP_TOOLCHAIN`, иначе `nightly`) и экспортирует его как
`RUSTUP_TOOLCHAIN`; экспортирует `CC_<triple>`, `CXX_<triple>`, `AR_<triple>`,
`CARGO_TARGET_<TRIPLE>_LINKER` и `WINDRES` для mingw-инструментов таргета;
печатает точную cargo-команду; после `build` читает subsystem-версию из
PE-заголовка каждого `.exe` (сам, `struct.unpack_from` по optional header).
`--target` по умолчанию `both`, пакеты — `horizon-legacy-render` и
`horizon-render-api`, профиль — `dev` (`fast` — играбельный). Коды возврата:
0 успех, 1 сборка упала, 2 аргументы, 3 нет компилятора, линкера или windres.

Сырой эквивалент — то, что скрипт печатает:

```sh
RUSTUP_TOOLCHAIN=nightly ~/.cargo/bin/cargo -Zbuild-std=std,panic_abort \
  build --profile fast --target i686-win7-windows-gnu \
  -p horizon-game --features legacy-gl
```

Без экспортированных `CC_*` эта команда падает: cc-rs для `hidapi` берёт `cc`
из `PATH` и компилирует `etc/hidapi/windows/hid.c` хост-компилятором. Именно
поэтому скрипт, а не сырой cargo.

`RUSTUP_TOOLCHAIN=nightly` и `~/.cargo/bin/cargo` важны: на этой машине `cargo`
из `PATH` — это `/usr/bin/cargo` (stable 1.100.0 из дистрибутива), и он
отвергает `-Z` с сообщением «the `-Z` flag is only accepted on the nightly
channel». `~/.cargo/bin/cargo` — rustup-прокси, он выбирает nightly по
`RUSTUP_TOOLCHAIN`.

Отдельный probe (`tools/legacy-gl-probe`, свой workspace без игрового графа
зависимостей) собирается без скрипта:

```sh
RUSTUP_TOOLCHAIN=nightly ~/.cargo/bin/cargo -Zbuild-std=std,panic_abort \
  build --release --target i686-win7-windows-gnu \
  --manifest-path tools/legacy-gl-probe/Cargo.toml
```

Две зависимости починили на `mvp2-win7-build`, чтобы GNU-линковка вообще
прошла; без них `horizon-game` падает, а не собирается:

- `Cargo.lock`: `os_pipe` тянул `windows-sys` 0.45, у которого единственный
  путь без raw-dylib — `#[link(name = "windows")]`, MSVC-импорт-библиотека,
  которой mingw-w64 не поставляет; каждая GNU-линковка падала с
  `cannot find -lwindows`. 0.52 линкует `windows.0.52.0`, который в mingw есть.
- `horizon-threads/src/work.rs`: множитель id-кэша был 64-битным литералом
  `usize` — на i686 это hard error (`overflowing_literals`). Теперь константа
  усекается до ширины указателя.

Плюс `wgl.rs`: `fetch_update` → `try_update` (stable с 1.95, `rust-version`
workspace'а) — это было единственное `deprecated`-предупреждение Windows-сборки.

CI этот путь не проверяет: ни один из 22 workflow в `.github/workflows` на
`mvp2-win7-build` не упоминает `win7_build.py` или `*-win7-windows-gnu`, так что
Windows-сборка снова сломается молча. Если её нужно держать зелёной, дешёвый
шаг — job с `--check -p horizon-game --features legacy-gl`, затем проверка, что
`pe_subsystem()` каждого exe ≤ (6, 1). Это предложение, а не существующий job.

### 19.4. Subsystem-версия PE: измерено, не предположено

Загрузчик Win7 отказывает PE, у которого subsystem version выше 6.1; это поле —
*минимальная* версия ОС, а не та, под которую собран код. Поэтому малое значение
здесь — хорошо, а не подозрительно (в первой версии скрипта вывод читался
наоборот; исправлено в `92c2990ec87`). Измерено `objdump -p <exe>` и тем же
измерением, что печатает скрипт:

| exe | subsystem | PE | грузится на Win7 |
| --- | --- | --- | --- |
| probe `i686-win7-windows-gnu` | **4.0** | PE32 | да |
| probe `x86_64-win7-windows-gnu` | **5.2** | PE32+ | да |
| `horizon-client.exe` i686 (`--profile fast`, 214 МиБ) | **4.0** | PE32 | да |
| `horizon-client.exe` x86_64 (`--profile fast`) | **5.2** | PE32+ | да |

То есть эти таргеты уже объявляют 4.0/5.2, ниже 6.1, — отдельного линкерного
флага для subsystem не требуется, ни для probe, ни для игры. Проверять это
после каждой сборки дешево: `file` печатает `PE32 executable for MS Windows
4.00` прямо в первой строке.

### 19.5. CRT и API-sets: вот что реально ломает запуск на Win7

Subsystem-версия — не единственное условие. Все четыре exe импортируют UCRT
API-set — `api-ms-win-crt-heap-l1-1-0.dll` и другие: восемь наборов у
64-битного probe, девять у 32-битного и девять у игры. mingw-w64 здесь собран на
UCRT (`/usr/i686-w64-mingw32/lib/libmsvcrt.a` побайтно совпадает с `libucrt.a`).
На Win7 UCRT появляется только вместе с обновлением KB2999226 или старым
vcredist; на голой системе его нет.

Это не решается одним флагом. `-mcrtdll=msvcrt-os` линкует обычную C-программу
против `msvcrt.dll`, но для Rust-std `-C link-arg=-mcrtdll=msvcrt-os` результат
не меняет: std уже собран против UCRT-импортов mingw-w64, и набор `api-ms-win-*`
остаётся в таблице импорта. `+crt-static` тоже не помогает. Убрать его можно
только своим mingw-w64, собранным с `--with-default-msvcrt=msvcrt`, чего на этой
машине нет. Поэтому:

- `python3 tools/check_imports.py <exe>` проходит (`exit 0`), но его allowlist —
  это Windows 10/11 (`tools/check_imports.py:18`), где `api-ms-win-*` есть, и
  **не** доказательство Win7-совместимости;
- для Win7 у него теперь есть `--windows7`: он отвергает WinRT
  (`api-ms-win-core-winrt*`), `PathCch*` (`api-ms-win-core-path`) и `xinput1_4.dll`
  как отсутствующие на Win7, а `api-ms-win-crt-*` печатает как требующие
  KB2999226 — это предупреждение, а не провал проверки, потому что UCRT
  устанавливается, а WinRT — нет. На собранном exe это измеряемо в CI, без самой
  Win7: `python3 tools/check_imports.py --windows7 <exe>`; `tools/win7_build.py`
  запускает его сам после каждой сборки и валит сборку только на первом списке;
  тесты — `tests/test_check_imports.py`.
- честный статус CRT на сегодня — **не проверено**; на целевой машине нужен
  либо KB2999226, либо mingw-сборка на `msvcrt.dll`.

У игры есть **вторая, независимая** проблема того же рода. Её `horizon-client.exe`
статически импортировал три API-set DLL, которых нет ни в одном probe:

| DLL | импортируемый символ | откуда | статус |
| --- | --- | --- | --- |
| `api-ms-win-core-path-l1-1-0.dll` | `PathCchStripPrefix` | `arboard` 3.7 → `windows-sys` 0.59 (`horizon-client-input`, буфер обмена) | **убрано**: на Windows текст берётся через `clipboard-win` (§19.9) |
| `api-ms-win-core-winrt-l1-1-0.dll` | `RoGetActivationFactory` | `windows` 0.62 / `windows-core` 0.62: `gilrs`, `horizon-dualsense`, `horizon-narrator`, затем `wgpu` (`wgpu-hal`, `gpu-allocator`) | **убрано**: три `#[cfg]` на `target_vendor = "win7"` плюс вендоренный `windows-core` (§19.9) |
| `api-ms-win-core-winrt-error-l1-1-0.dll` | `RoOriginateErrorW` | `windows-result` 0.4.1 (`windows-core` 0.62) | **убрано**: `--cfg=windows_slim_errors` (§19.9) |

Первая строка таблицы — уже измерение, а не вывод: сборка после замены буфера
обмена печатает таблицу импорта без `api-ms-win-core-path` вовсе
(`i686-w64-mingw32-objdump -p`), а два WinRT-импорта на тот момент ещё стояли.
`api-ms-win-winrt-error` снимает `windows_slim_errors`, который
`tools/win7_build.py` ставит через `CARGO_TARGET_<triple>_RUSTFLAGS`; в
`windows-result` 0.1.2 (её тянет только `cpal` 0.16) такой ручки нет — это
единственное, что удерживает этот импорт, и закрывается оно bump'ом `cpal`,
который отложен по другой причине (смена API, §19.9).

Delay-import directory во всех четырёх exe пуст (`objdump -p` печатает
`Entry d 00000000 00000000 Delay Import Directory`), то есть это статические
импорты: загрузчик Windows ищет эти DLL до первой инструкции программы.
`api-ms-win-core-winrt-*` — это WinRT, которого на Windows 7 нет вообще, а
функции `PathCch*` появились в Windows 8. Значит, сегодняшняя игра не
загрузится на Win7 даже с установленным UCRT. Это вывод из таблицы импорта
(измерено) плюс известные версии, в которых эти API появились; на настоящей Win7
не проверялось, и в §19.7 он стоит как непроверенный.

Заполнить измерением: собрать probe (у него этих трёх импортов нет) и прогнать
на чистой Win7 SP1 x86 без обновлений; если он не стартует — это UCRT.

### 19.6. WGL под Wine: проходят оба варианта

`tools/legacy_wgl_wine.py` (ветка `mvp2-wine-wgl`, в `mvp2` пока нет) собирает
probe под Win7-таргет, создаёт свежий `mktemp`-Wine-префикс (никогда `~/.wine`),
пинит его на Windows 7 через `HKCU\Software\Wine\Version=win7` и проверяет
`reg query`, поднимает постоянный виртуальный десктоп Wine и прогоняет два
случая (GL 2.1/GLSL 120 и GL 2.0/GLSL 110) через настоящий
`Context::new("wgl", …)`, а не ручную настройку WGL.

На этой машине (Wine 11.22, Mesa llvmpipe) прогон 2026-11-18:

| цель | результат |
| --- | --- |
| `x86_64-win7-windows-gnu`, Wine | **passed** оба случая: `context = WGL desktop OpenGL window`, `buffer_swap = true`, `pointer_bits = 64`, `gl_version = 2.1 / 2.0 Mesa 26.2.6-arch3.1`, `glsl_version = 1.20`, renderer `llvmpipe (LLVM 23.1.4, 256 bits)`, 7 checks и 25 backend outcomes |
| `i686-win7-windows-gnu`, Wine | **passed** оба случая, те же строки, `pointer_bits = 32`, 7 checks и 25 backend outcomes |

32-бит — это форма реальной машины X1650, так что прошедший i686-прогон важнее
64-битного: harness собирает probe именно под `i686-win7-windows-gnu` и запускает
его в `WINEARCH=win64`-префиксе Wine, где 32-битные процессы идут через WoW64.

Запуск нашёл два настоящих бага WGL, оба исправлены на `mvp2-wine-wgl`:

- pixel format просил 24 бита цвета и ни одного alpha, поэтому `glReadPixels`
  отдавал alpha 255 везде и все проверки полупрозрачных наложений падали. Теперь
  просится 32 бита с 8-битным destination alpha, как у EGL- и GLX-конфигов, а
  формат без него отвергается.
- `resize()` двигал окно, но не GL-drawable: Wine пересоздаёт drawable из
  client rect только на present, поэтому GL продолжал рисовать в старый буфер, а
  новая область читалась нулями. Теперь после `SetWindowPos` делается один
  present, под проверкой живого контекста.

Что доказывает прогон: настоящий `wglCreateContext` на `CS_OWNDC`-окне,
разрешение символов через `wglGetProcAddress` с fallback в `opengl32.dll`
(`crates/horizon-legacy-render/src/wgl.rs:516`) — `glCreateShader`,
`glShaderSource` и `glGenBuffers` Wine не экспортирует из `opengl32.dll`, так
что прошедший кадр доказывает и этот fallback, — present и readback пикселя в
Windows-PE-процессе, на 32 и 64 битах.

Что не доказывает: X1650 и её ICD. Wine отдаёт Mesa, а не драйвер ATI;
`MESA_GL_VERSION_OVERRIDE` прибивает *объявленную* версию, а не поведение
старого драйвера. И это probe, не игра: полный exe под Wine не запускался.

### 19.7. Что проверено, а что нет

| Утверждение | Статус |
| --- | --- |
| `horizon-legacy-render` компилируется под `x86_64-win7-windows-gnu` | **да**, `check -p horizon-legacy-render` |
| он же под `i686-win7-windows-gnu` | **да**, там же |
| probe линкуется в PE32/PE32+ с subsystem 4.0/5.2 | **да**, `objdump -p` |
| полная игра `horizon-game --features legacy-gl` собирается | **да**, `tools/win7_build.py --profile fast`, оба таргета; exe 214 МиБ |
| её exe тоже объявляет 4.0/5.2 | **да**, тем же измерением скрипта |
| subsystem ≤ 6.1 гарантирует запуск на Win7 | **нет**: CRT — это UCRT (§19.5) |
| WGL-контекст и кадр под Wine, 64-bit | **да**, §19.6 |
| WGL-контекст и кадр под Wine, 32-bit | **да**, §19.6 |
| игра загрузится на Win7 с установленным UCRT | **вероятно, да**: таблица импорта больше не содержит WinRT и `PathCch*` (проверено `check_imports.py --windows7`), остаются только `api-ms-win-crt-*` за KB2999226 (§19.5); на настоящей Win7 не запускалось |
| запуск на настоящем железе Win7 / X1650 | **не проверялось никогда** |

### 19.8. Что нужно на реальной Win7

- Windows 7 **SP1**, 32-битная сборка игры (X1650-класс — 32-битный сценарий).
- Драйвер производителя с аппаратным ICD для desktop GL 2.0/2.1. `Microsoft GDI
  Generic` — это отсутствие ICD, а не GL2: `driver.rs:4` и `wgl.rs:361` отвергают
  такой pixel format с внятной ошибкой вместо падения.
- Никаких SDK, Rust или современного launcher на самой машине: exe + ресурсы.
  Ресурсы готовятся на современном компьютере из своей копии игры
  ([§8](#8-приёмка-и-риски)); Mojang-данные не входят в архив.
- Контекст создаётся через legacy `wglCreateContext` (`wgl.rs:375`), без
  `WGL_ARB_create_context`/core profile — проверено чтением кода, ни одного
  `CreateContextAttribs` в крейте нет. Это ровно то, что даёт драйвер Win7-эпохи.
- `KB2999226` (Universal C Runtime) — иначе не загрузятся `api-ms-win-crt-*`
  импорты Rust-std (§19.5). Это минимум и для probe, и для игры.
- WinRT-импортов и `PathCch*` в таблице импорта игры больше нет: `clipboard-win`
  вместо `arboard`, три `#[cfg]` на `target_vendor = "win7"` и два вендоренных
  `windows-core` (§19.9). Проверяется без самой Win7:
  `python3 tools/check_imports.py --windows7 <exe>`; `tools/win7_build.py`
  запускает эту проверку сам после каждой сборки (§19.5).

Запуск игры на таком железе — отдельный gate T02/O02; успешный Wine-прогон его
не закрывает.

### 19.9. Убирание трёх импортов: что проверено на опыте

Всё ниже измерено на этой машине (`mingw-w64` 2.47, nightly + `-Zbuild-std`) на
маленьких программах, повторяющих зависимость игры, а не по документации.

**`api-ms-win-core-path-l1-1-0.dll` (`PathCchStripPrefix`) — убран.** Импорт
приходил целиком из `arboard`: он нужен только для `set_files` (список файлов
для drag-and-drop), а игра читает и пишет только текст. `arboard` 3.6.1 и 3.7.0
(последняя на crates.io) обе импортируют его, и `default-features = false` не
помогает: символ линкуется, даже когда до `set_files` нет вызовов — линкер
тянет весь объектный файл `platform/windows.rs`. Замена на Windows —
`clipboard-win` (та же зависимость, что уже стоит под `arboard`): программа,
которая вызывает только `get_clipboard_string`/`set_clipboard_string`, не имеет
этого импорта вовсе (проверено `objdump -p`). Так `clipboard.rs` и разделён:
`arboard` на `cfg(not(windows))`, `clipboard-win` на `cfg(windows)`.

**Обход через подмену IAT-слота не работает.** Идея — определить в своей
программе `__imp_RoGetActivationFactory` и тем не дать линкеру взять импорт из
`raw-dylib`. Проверено на настоящем `gilrs` (backend `wgi`): линковка падает с
`multiple definition of '_imp_RoGetActivationFactory'` — `windows-core` сам
определяет этот слот в объектнике `api-ms-win-core-winrt-l1-1-0.dlls00000.o`, и
ld не разрешает перекрыть его определением из другого объекта. Так что этот
путь закрыт; WinRT-импорты убираются только правкой зависимостей.

**`windows_slim_errors` убирает `RoOriginateErrorW`, но не
`RoGetActivationFactory`.** `windows-result` 0.4.1 под этим `--cfg` (документирован
в самом крейте) выбрасывает код `IErrorInfo`, и импорт
`api-ms-win-core-winrt-error-l1-1-0.dll` из программы исчезает — проверено.
`RoGetActivationFactory` остаётся: его тянет `windows-core`'s `FactoryCache`, а он
собирается всегда, как только в графе есть `windows-*`, даже если ни одна
WinRT-фабрика не запрашивается (программа, которая только читает `GUID`, всё равно
получает этот импорт).

**Остаток — WinRT через `windows-core`/`windows`.** Сейчас его тянут
`gilrs` (backend `wgi`, `windows` 0.62), `cpal` WASAPI (`windows` 0.54) и
`horizon-dualsense` (`windows` 0.62). У `gilrs` есть второй backend на XInput:
`gilrs = { default-features = false, features = ["xinput"] }` собирается под тот же
таргет и **не** импортирует `api-ms-win-core-winrt-*` (проверено). `cpal` 0.16
(текущая версия в дереве) использует только `Win32::Media::Audio` и WinRT-классов
не активирует, но `windows-core` 0.54 всё равно оставляет импорт
`RoGetActivationFactory`. **`cpal` 0.18 снимает это**: она на `windows` 0.62, и
программа, которая только открывает устройство вывода, оставляет ровно один
WinRT-импорт — `api-ms-win-core-winrt-error-l1-1-0.dll`, — а он уходит под
`windows_slim_errors` (проверено: `NONE`).

`tools/win7_build.py` ставит `--cfg=windows_slim_errors` через
`CARGO_TARGET_<triple>_RUSTFLAGS`, так что он применяется ко всем крейтам графа.
Это снимает `RoOriginateErrorW` без правки зависимостей.

**Что сделано с `RoGetActivationFactory`: два `#[cfg]`, по одному на
потребителя.** Оба используют `target_vendor = "win7"` — свойство самого
Rust-таргета `*-win7-windows-gnu`, поэтому обычные Windows-сборки не меняются
(проверено: Cargo действительно исключает зависимость под
`cfg(not(target_vendor = "win7"))` на хосте и включает под win7-таргетом).

- `gilrs` в `horizon-client` и `horizon-client-lib` объявлен дважды: обычный
  Windows/macOS/Linux — backend по умолчанию (`wgi` на Windows), а под
  `cfg(target_vendor = "win7")` — `default-features = false, features = ["xinput"]`.
  Backend'ы взаимоисключающие (`compile_error!` в `gilrs-core`, если включены
  оба), поэтому это две декларации, а не одна с фичами. Публичный API `gilrs`
  от backend'а не зависит — ни одна строка игры не меняется: `hardware.rs`
  называет `gilrs::Gilrs`, `gilrs::Gamepad`, `gilrs::ff::Effect` одинаково на
  обоих. `rusty-xinput` грузит `xinput1_4/1_3/1_2/1_1/9_1_0.dll` через
  `LoadLibraryW` в рантайме (статического импорта нет), а `xinput9_1_0.dll`
  есть в Windows 7 SP1. XInput видит пады Xbox 360 и Xbox One; что теряется —
  пады, которые видит только Windows.Gaming.Input (часть DualShock/DualSense
  через WGI), и всё, что Win7 и так не умеет.
- `horizon-dualsense`: модуль `xbox::wgi` и зависимость `windows` объявлены под
  `cfg(all(target_os = "windows", not(target_vendor = "win7")))`. На Win7
  Windows.Gaming.Input нет вообще, так что это мёртвый код: там пад берётся
  только через hidapi, и у проводного Xbox-пада импульсных курков не будет
  (`output_supported` уже это и говорит). Вариант `Error::Wgi` тоже под тем же
  `cfg`.

**Вендоренный `windows-core`: последний `RoGetActivationFactory` приходит из `wgpu`.**
После двух `#[cfg]` в exe оставался ровно один WinRT-импорт, и он не из
`gilrs`/`dualsense`/`narrator`: `cargo tree -i windows@0.62.2` на win7-таргете
показывает только `wgpu-hal` и `gpu-allocator`, а оба приходят из `wgpu`.
`windows-core` 0.62.2 линкует `api-ms-win-core-winrt-l1-1-0.dll` **безусловно**
(`src/imp/bindings.rs`), а `pub fn load_factory` в `src/imp/factory_cache.rs` —
единственный, кто её вызывает; поэтому удаление мёртвого кода её не снимает.
Никакой набор фич `wgpu` этого не обходит: `dx12`, `gles` и `vulkan` на Windows
все тянут `windows` 0.62 (включены только `Win32_*`-фичи, ни одного WinRT-
namespace, но импорт всё равно линкуется).

Поэтому `third_party/windows-core` — вендор 0.62.2 с одним изменением: под
`cfg(target_vendor = "win7")` строка `RoGetActivationFactory` в
`src/imp/bindings.rs` не объявлена, а `load_factory` возвращает
`REGDB_E_CLASSNOTREG` (Win7 и не умеет активировать WinRT-класс; ничего в
Win7-сборке его не активирует). Крейт подменён в `[patch.crates-io]`, вне
`members` и с записью в `THIRD_PARTY_NOTICES.md`, как `wgpu-hal` и `naga`.
Проверено: `nm` вендоренного rlib под `i686-win7-windows-gnu` не содержит
`RoGetActivationFactory` (остаётся только `UuidCreate` из `rpcrt4.dll`, которая
на Win7 есть), а импорт-таблица собранного exe — см. §19.8.

Но 0.62.2 — не единственная копия крейта в графе: `cpal` 0.16 идёт через
`windows` 0.54 → `windows-core` 0.54.0, у которого та же безусловная строка
(в `src/imp/bindings.rs`, `factory()` вместо `load_factory`). Поэтому
`third_party/windows-core-0.54` — вендор 0.54.0 с тем же изменением, и в
`[patch.crates-io]` он стоит отдельным ключом (`windows-core-v054`): запись
`[patch]` подменяет одну версию пакета за раз, поэтому две версии — два ключа.
Альтернатива, bump `cpal` до 0.18, сняла бы 0.54-линию целиком, но это смена
API (§ниже), отложенная до появления звукового устройства для проверки.

**`cpal` 0.18 отложен, не потому что не работает.** Проверено, что 0.18
снимает WinRT-импорт, но это не подъём версии, а смена API: `build_output_stream`
берёт `StreamConfig` по значению и возвращает `cpal::Error` вместо
`BuildStreamError`, `Device::name()` заменён на `description() -> DeviceDescription`,
`StreamError` стал `Error`, `SampleRate` — простой `u32` вместо newtype.
`audio.rs` держит на этом API восстановление умершего ALSA-стрима
(`StreamErrors`, POLLERR, повторное открытие с удлинённым буфером), и проверить
это без звукового устройства нельзя — это отдельная работа, а не правка ради
импорта. Пока WinRT убирают три `#[cfg]` на `target_vendor = "win7"` и два
вендоренных `windows-core` выше.



