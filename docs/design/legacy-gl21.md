# План: OpenGL 2.1/2.0 и запуск на AMD Radeon X1650

Статус: PR 0–10 реализованы аддитивно, затем по запросу «Теперь интегрируй все в игру» (`full_pr8` + `real_gl`) выполнена полная интеграция: `#[cfg]`-разбор `gfx`/`game.rs`/`build_renderer`/`loading_renderer`, `legacy-gl` без `modern-gl`, `GraphicsApi::LegacyGl`, UI/локализация/`--legacy-gl`, собственный CI и реальный кадр GL2 через glutin/glow с terrain/entity/sky/HUD/lines. После «Продолжи реализацию» выбран объём `full`; PR 11 добавляет интерактивный одиночный GL2-клиент со стримингом чанков, физикой, редактированием мира, хотбаром, меню, инвентарём, сохранением и динамической сценой. PR 12 добавляет авторитетный интегрированный сервер на отдельном потоке, команды/снапшоты, LAN-сервер и LAN-клиент на внутреннем JSON-протоколе, `--integrated`, `--lan-port`, `--join`, `--name`, сетевой HUD и 14 шагов CI. Это не Minecraft Java protocol; полный порт 22k-строчного `game.rs`, dedicated server, wgpu-паритет и запуск на реальной X1650 вне объёма и не измерены.
Язык документа русский по запросу; остальные страницы `docs/` остаются английскими, поэтому в
[индексе документации](../README.md) ссылка помечена `(RU)`. Полные `cargo`-проверки вынесены в собственный
workflow ветки [`arena-legacy-gl.yml`](../../.github/workflows/arena-legacy-gl.yml): чужие actions, PR и workflow
не просматриваются и не изменяются.

Связанные страницы: [графика и настройки](../graphics.md), [нативный клиент](../native-client.md),
[шейдер-паки](../shaders.md), [архитектура](../architecture.md), [CI-раннеры](../ci-runners.md),
[правила для агентов](../../AGENTS.md), [CONTRIBUTING](../../CONTRIBUTING.md).

---

## 0. Короткий вывод

**Переписывать игру целиком не нужно.** Симуляция, миры, протокол, меню как данные и 300+ дифференциальных
харнессов не зависят от GPU. Переписывается только графический слой: вместо `wgpu` нужен отдельный рендерер на
«сыром» OpenGL 2.1/2.0, собранный в отдельном Cargo-фиче. Текущий GL-бэкенд wgpu на X1650 не поднимется вообще:
он требует OpenGL 3.3, а X1650 даёт максимум 2.1.

Рекомендуемая форма: **`#[cfg(feature = "legacy-gl")]`-алиас `Renderer` + новый модуль `gfx21`**, который повторяет
публичный контракт `gfx::Renderer` заглушками там, где функция физически недоступна. Отдельный бинарник
`horizon-client` остаётся тем же, меняется только реализация рендерера. Порядок fallback'ов:
`Vulkan → DX12 → wgpu GL (3.3+) → legacy GL 2.1 → CPU/llvmpipe`.

Цель первого релиза — **не паритет картинки и не 60 FPS**, а устойчивый запуск: титульный экран, загрузка мира,
террейн, сущности, небо/туман/лайтмап, HUD и меню на 800×600 при render distance 4–6 без `GL_ERROR`.

## 0.1 Статус внедрения

| PR | Состояние | Что сделано / что осталось |
|---|---|---|
| 0 | готово локально | этот документ и ссылка в [docs/README.md](../README.md); `git diff --check`, `tools/check_public_tree.py`. Issue + `/claim` на GitHub отложены до явного запроса, чтобы не создавать шум в чужом форке. |
| 1 | готово | `tools/gl21_caps.py`, `tests/test_gl21_caps.py`, фикстура `tools/gl21_caps_fixtures/x1650-catalyst93.glxinfo.txt`, эталон `benchmarks/graphics/x1650-reference-caps.{json,txt}` и шаблон `benchmarks/graphics/x1650-caps.TEMPLATE.md`. **Реальный дамп с X1650 и с Mesa `r300g` нужно снять на железе**: в песочнице карты нет. |
| 2 | 2a и 2b готовы | 2a: `gfx21/{mod.rs,caps.rs,CONTRACT.md}`, `pub mod gfx21;`, Python-тесты контракта. 2b: фичи `legacy-gl` в обоих `Cargo.toml`, `wgpu`/`naga`/`pollster` → optional за `default = ["modern-gl"]`, алиас `pub type Renderer`/`Frame` в `lib.rs` под `#[cfg]`. `gfx` и `game.rs` пока остаются без `#[cfg]`-разбора — это PR 8. |
| 3 | готово (stage 1 + 2) | `gfx21/{context.rs,state.rs,smoke.rs}` остаются чисто-Рустовыми; `examples/gl21_clear.rs` под `required-features=["legacy-gl"]` создаёт GLX/WGL-контекст OpenGL 2.1/2.0 через `glutin`, грузит `glow`, читает `glGet*`, линкует треугольник (с PR 4 — весь набор программ) и проверяет `GL_NO_ERROR`. Опциональные зависимости и [нотисы](../../THIRD_PARTY_NOTICES.md) добавлены. |
| 4 | готово | `gfx21/program.rs` и 24 файла `gfx21/shaders/*`: шесть программ (smoke/terrain/entity/sky/HUD/line) в `#version 120`/`#version 110`, `validate_all()`, `program_for()`/`all_programs()`. Пример линкует весь набор в обоих Xvfb-кадрах; F0 остаётся без GLSL. |
| 5 | готово | `gfx21/{atlas.rs,mesh.rs}`: один 2D-атлас ≤2048² с гуттерами, half-texel UV и безопасными mip-уровнями; `TerrainVertex → GlVertex` с `UV_STEPS`/`TILED_UV_STEPS`, маской `OPAQUE_LAYER`, RGBA8, lightmap UV и явной 36-байтной записью. |
| 6 | готово | `gfx21/{sections.rs,passes.rs,frame.rs}` и [`tools/legacy_gl_golden.py`](../../tools/legacy_gl_golden.py): opaque/cutout/translucent, `glMultiDrawElementsEXT`-группы, туман/лайтмап, ≤200 draw calls, CPU golden [`PNG`](../../benchmarks/graphics/legacy-gl-scene.png)/RGBA 80×60. |
| 7 | готово аддитивно | `gfx21/{blend,entities,sky,hud,lines,render}.rs`: шесть GL2 blend/alpha-test режимов и `SortState`, CPU-posing сущностей, небо/туман/день-ночь, HUD title/inventory/chat/hotbar, линии и `FullFramePlan` на 16 draw calls; PR 6 golden-кадр не изменён. |
| 8 | готово (полная интеграция) | `gfx21/facade.rs` плюс `#[cfg]`-разбор `lib.rs`/`client_lib.rs`/`game.rs`, `game_legacy.rs`, `legacy_gl.rs`, `GraphicsApi::LegacyGl`, ключи UI/локализации, `horizon-graphics.txt`, `--legacy-gl`, реальный GL2-кадр и `legacy-gl` без `modern-gl`. |
| 9 | готово аддитивно | `gfx21/diagnostics.rs`, [`tools/legacy_gl_smoke.py`](../../tools/legacy_gl_smoke.py), [`tests/test_gl21_smoke.py`](../../tests/test_gl21_smoke.py), [`stats.json`](../../benchmarks/graphics/legacy-gl-scene.stats.json) и [`startup report`](../../benchmarks/graphics/legacy-gl-startup-report.txt): стартовый отчёт, бюджеты draw/sampler/VRAM, имена `glGetError` и debug-лог. wgpu-diff не делается. |
| 10 | готово аддитивно | [README.md](../../README.md), [docs/native-client.md](../native-client.md), [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md), [CONTRACT.md](../../crates/horizon-client-render/src/gfx21/CONTRACT.md), план и свой workflow. Реальный X1650 и 30-минутный прогон не проводились. |
| 11 | готово (первый этап `full`) | `crates/horizon-client/src/legacy_game/`: детерминированные чанки, физика игрока, Raycast/break/place, хотбар, меню/пауза/инвентарь/настройки, HUD/FPS/день-ночь, CPU-растеризация текста, JSON-сохранение; `--game`, `--frames 0`, `--seed`, `--world`, `--fov`, `--sensitivity`; динамические VBO чанков и настоящий интерактивный GL2-кадр. |
| 12 | готово (этап сети) | `legacy_game/server.rs` и `legacy_game/net.rs`: авторитетный 20 Hz integrated server, `ServerCommand`/`ServerEvent`, снапшоты чанков, пауза/сохранения, внутренний LAN JSON-протокол на std TCP, `--integrated`, `--lan-port 0`, `--join`, `--name`, loopback Rust-тесты, HUD статуса сети и интегрированные GL2-кадры в CI. |


**Ограничение среды и CI.** Изначально в песочнице Arena не было `rustc`/`cargo`; теперь установлен нативный
Rust 1.99 (npm-пакеты `@toolchain-bin/*`, системный `x86_64-linux-gnu-ld.bfd`), но crates.io и
`static.rust-lang.org` отсюда недоступны. Поэтому полный `cargo check -p horizon-client` перенесён в
[`arena-legacy-gl.yml`](../../.github/workflows/arena-legacy-gl.yml) — свой workflow только для ветки
`arena/81644160-horizon`: на ранере GitHub ставятся Rust 1.99 (минимум 1.95 требует `horizon-mods`) и зависимости, затем выполняется gate PR 2b.
Локально проверены аддитивные модули отдельным крейтом без внешних зависимостей: `cargo test` (78 тестов),
`cargo clippy --offline --all-targets -- -D warnings`, `rustfmt --check` для `gfx21/*.rs` и примера;
`GFX21_GOLDEN_OUT=/tmp/legacy-gl-scene.rgba cargo test` пишет кадр, а
`python3 tools/legacy_gl_golden.py --check` сравнивает его с независимым Python-рендером;
плюс `python3 -m unittest tests.test_gl21_caps tests.test_gl21_contract tests.test_gl21_golden tests.test_gl21_smoke tests.test_gl21_integration tests.test_legacy_game tests.test_legacy_server -v` (64 теста),
полный `python3 -m unittest discover -s tests` (691 тест, 26 skipped; прямые `rustc`-пробы идут через wrapper с `-C linker-features=-lld` для локального Rust 1.99), `tools/gl21_caps.py --check` (L1),
`git diff --check`, `tools/check_public_tree.py`, 0 битых Markdown-ссылок.
Попытка собрать локальное зеркало из 3413 вендорённых крейтов GitHub покрыла 294/488 точных версий
`Cargo.lock`, но 194 (включая `cpal`, `glam`, `winit`) с разрешённых хостов недоступны; `cargo check` поэтому
делает CI. Новые опциональные `glow`/`glutin`/`glutin-winit`/`raw-window-handle` описаны в
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md). Свой workflow
[`arena-legacy-gl.yml`](../../.github/workflows/arena-legacy-gl.yml) зелёный: все 14 шагов, включая gate полного PR 8, проверку дерева зависимостей без `modern-gl`, golden-кадр, два smoke-кадра и два полных игровых кадра GL 2.1/2.0, проходят на Rust 1.99. Реальный дамп `glxinfo` с X1650/`r300g`
по-прежнему нужно снять на железе.

---

## 1. Целевое железо: что именно умеет X1650

Radeon X1650 — это R500/RV535, 2006–2007 год, типично 256–512 MiB VRAM, AGP или PCIe, в паре с двухъядерным CPU
и 2–4 GiB ОЗУ. Дамп возможностей Catalyst 9.3 на Windows 7 для `Radeon X1600/X1650 Series`:
<https://vgamuseum.info/images/doc/ati/x1650.txt>. Mesa с gallium-драйвером `r300` (R300–R500) тоже даёт OpenGL 2.1 /
GLSL 1.20, но не GL 3.x: <https://wiki.archlinux.org/title/OpenGL>,
<https://mesa-docs.readthedocs.io/_/downloads/en/latest/pdf/>.

### 1.1 Профили

| Профиль | Контекст | GLSL | Где встречается | Что делаем |
|---|---|---|---|---|
| **L1** (основной) | OpenGL 2.1 | `#version 120` | Catalyst 9.3 legacy (XP/Vista/7), Mesa `r300g` на X11/GLX | основной путь |
| **L0** | OpenGL 2.0 | `#version 110` | более старые Catalyst, часть fglrx | тот же рендерер, второй вариант шейдеров |
| **F0** (аварийный) | GL 1.5 + ARB VBO/ARB vertex+fragment program или fixed-function | без GLSL | драйверы со сломанным GLSL-компилятором | упрощённая картинка, но гарантированный запуск |

Современные Windows 10/11 для R500 драйвера с OpenGL 2.x обычно не имеют (только Microsoft Basic Display,
OpenGL 1.1) — **реалистичная платформа №1: Linux x86-64 + Mesa `r300g` + X11/GLX**. Windows 7 + Catalyst 9.3 —
платформа №2. macOS не цель.

### 1.2 Что есть (по дампу Catalyst 9.3)

| Возможность | Значение | Как используем |
|---|---|---|
| `GL_VERSION` / GLSL (дамп) | 2.1.8545 / 1.20 | `#version 120`, fallback `#version 110` |
| Multitexture Texture Units (дамп; combined = 16) | 8 | ≤4 сэмплера на проход |
| Max Vertex Attributes (дамп) | 16 (нативный предел интерполяторов ≈10) | ≤8 атрибутов |
| Max Uniform Vertex Components (дамп) | 512 (=128 `vec4`) | дробить `Globals`, см. §4.4 |
| Max Varying Floats (дамп) | 44 (=11 `vec4`) | pos/uv/color/light/fog хватает |
| Max Texture Size (дамп) | 4096 | атлас 2048² |
| Max Draw Buffers (дамп) | 4 | MRT не нужен |
| Max Vertex Texture Image Units (дамп) | **0** | **никаких VTF, никаких storage-буферов в вершинном шейдере** |
| Расширения | `GL_EXT_framebuffer_object`, `GL_ARB_framebuffer_object`, `GL_EXT_framebuffer_blit`, `GL_EXT_packed_depth_stencil`, `GL_ARB_depth_texture`, `GL_ARB_vertex_buffer_object`, `GL_ARB_pixel_buffer_object`, `GL_ARB_texture_non_power_of_two`, `GL_EXT_multi_draw_arrays`, `GL_ARB_occlusion_query`, `GL_ARB_shading_language_100`, `GL_EXT_texture_filter_anisotropic`, `GL_ARB_shader_texture_lod`, `GL_ARB_map_buffer_range`, `GL_ARB_vertex_array_object`, `GL_EXT_texture_sRGB`, `WGL_EXT_framebuffer_sRGB` | FBO/глубина, VBO/PBO, `glMultiDraw*`, CPU-анимации атласа, опциональный explicit-LOD |

В дампе есть строка `Max Texture Array Layers 2048`, но в списке расширений нет `GL_EXT_texture_array` /
`GL_TEXTURE_2D_ARRAY`, а GL 3.0 помечен как «No (38%)»: на массивные текстуры рассчитывать нельзя.
Точно так же не стоит полагаться на `GL_ARB_vertex_array_object` на Mesa `r300g`: VAO считаем опцией.

### 1.3 Чего нет (и это ломает текущий рендерер)

| Отсутствует | Почему важно |
|---|---|
| GL 3.0+, core/compatibility profile, `glGenVertexArrays` как базовый путь | wgpu GL-бэкенд требует 3.3 |
| `GL_EXT_texture_array`, `GL_ARB_texture_buffer_object` | `terrain.wgsl` биндит до 8 `texture_2d_array` |
| UBO/SSBO, `GL_EXT_shader_image_load_store`, atomics | GPU-culling, packed arena, OIT, visbuffer |
| `GL_ARB_draw_indirect`, `GL_ARB_shader_draw_parameters` | один indirect multi-draw на проход |
| `GL_ARB_instanced_arrays`, `gl_VertexID`, `gl_InstanceID` | flat-проходы, `gl_VertexIndex` в шейдерах |
| compute shaders | анимация спрайтов, скининг, frame generation, GPU-каллинг |
| целочисленные атрибуты, `uint`/битовые операции в GLSL 1.20 | `TerrainVertex.layer: u16`, `light: u16`, `packed::` |
| `texelFetch`, `textureSize`, `textureGrad`, `textureLod` (кроме ARB-расширения) | `sample_nearest`, RGSS, mip-логика |
| `R32Float`-атомика, `RGBA32F`-blending, HDR swapchain | OIT, HDR, FSR |
| `GL_ARB_timer_query` (нет в списке) | GPU-тайминги |
| `glClipControl`, `glTexStorage`, `glMultiDrawIndirect` | мелочи, но их надо заменить |

---

## 2. Почему «просто переключить Graphics API» не сработает

Сегодня клиент — это `wgpu` 30.0.1 ([Cargo.toml](../../Cargo.toml),
[`crates/horizon-client-render/Cargo.toml`](../../crates/horizon-client-render/Cargo.toml),
[`crates/horizon-client/Cargo.toml`](../../crates/horizon-client/Cargo.toml)), вариант `GraphicsApi::OpenGl`
выбирает `Backends::GL` в [`gfx.rs:1637`](../../crates/horizon-client-render/src/gfx.rs).

1. **wgpu GL-бэкенд жёстко требует 3.3.** В вендорённом
   [`third_party/wgpu-hal/src/gles/adapter.rs:268`](../../third_party/wgpu-hal/src/gles/adapter.rs):
   `Returned GL context is {}.{}, when 3.3+ is needed`. [README.md](../../README.md) прямо пишет «OpenGL 3.3 (Linux)».
   На X1650 адаптер не будет создан, и `open_device` уйдёт в llvmpipe, который на старом CPU неподъёмен.
2. **Транслятор шейдеров не умеет ниже GLSL 330.** Документация вендорённого naga
   ([`third_party/naga/src/back/glsl/mod.rs`](../../third_party/naga/src/back/glsl/mod.rs)): поддерживаются Desktop
   330–450 и ES 300/310. Тест в `gfx.rs:11209` уже фиксирует, что compute/storage-шейдеры «never run on GL», а
   GLSL ниже 4.3 не имеет `ARB_arrays_of_arrays`. GLSL 110/120 из WGSL автоматически не получить.
3. **Семантика WGSL невыразима в GLSL 1.10/1.20.** 29 `.wgsl`-файлов, 5373 строки. `terrain.wgsl` использует
   `texture_2d_array`, `var<storage>`, `u32`, `bitcast`, `@interpolate(flat)`, `textureSampleGrad`,
   `textureSampleLevel`, 128 `emissive_layers`, 8 спрайтовых массивов. `entity.wgsl`, `hud.wgsl`, `oit*.wgsl`,
   `visbuffer.wgsl`, `frame_gen/*.wgsl`, `hdr*.wgsl` — то же плюс compute/атомика/float-таргеты.
4. **Объём связки с wgpu.** В `horizon-client-render` 212 `.rs`-файлов / 177 568 строк; 68 файлов содержат
   5756 обращений `wgpu::`. `gfx.rs` — 11 833 строки, подмодули `gfx/` — 41 420 строк, 80 публичных методов и
   107 публичных полей у `Renderer`. `game.rs` обращается к `renderer.config`, `.hud`, `.device`, `.queue`,
   `.info`, `.screenshot`, `.gpu_times`, `.last_wait`, `.last_encode`, `.drawn_sections`, `.capture_result`,
   `.pack_inputs` и ещё ~30 методам/полям ([`docs/architecture.md`](../architecture.md), строка «Renderer core»).
5. **Запрашиваемые фичи устройства** в `gfx.rs:1914` включают `MULTI_DRAW_INDIRECT_COUNT`,
   `INDIRECT_FIRST_INSTANCE`, `VERTEX_WRITABLE_STORAGE`, `TEXTURE_ATOMIC`, `FLOAT32_FILTERABLE`,
   `FLOAT32_BLENDABLE`, `SUBGROUP`, `TIMESTAMP_QUERY`, `CLEAR_TEXTURE`. Ни одной из них на R500 нет.
6. **Глубина**: везде `Depth32Float` + `CompareFunction::GreaterEqual` (реверсивный Z,
   [`camera.rs:665`](../../crates/horizon-client-render/src/camera.rs)). GL2 не имеет `glClipControl`; нужен
   отдельный разбор (§4.6).

### Отвергнутые альтернативы

| Идея | Почему нет |
|---|---|
| Сконфигурировать wgpu под GL 2.1 (`required_features = empty`) | ядро `gles`-бэкенда всё равно требует 3.3, а шейдеры транслируются в GLSL 330 |
| Zink / lavapipe / DXVK | для R500 нет Vulkan-драйвера, а софтверный рендер на старом CPU даёт единицы FPS даже в меню |
| llvmpipe как «просто запустить» | `device_search` уже разрешает CPU-адаптер последним шансом; это не «запуск на X1650» |
| Автотрансляция WGSL → GLSL 1.20 | naga не генерирует ниже 330, и `uint`/SSBO/массивы текстур не имеют аналога |
| Полностью новый клиент с нуля | дублирует 20k строк `game.rs`, стриминг, HUD, `horizon-ctl` и портит паритетные харнессы |

---

## 3. Архитектура решения

```
horizon-client (game.rs)  ──  pub type Renderer = …
                              ├── #[cfg(not(legacy-gl))]  gfx::Renderer      (wgpu, как сейчас)
                              └── #[cfg(legacy-gl)]       gfx21::Renderer    (GL 2.1/2.0)

Общий CPU-слой (без wgpu):
  mesher::SectionMesh / TerrainVertex   sprites::Sprites / mipmap
  gfx::translucency::Sorter            hud::HudBatch / HudVertex
  camera / light / fog / stars / sunrise_fan / clouds / weather (геометрия)
  graphics::Graphics + options.txt      Frame<'a> (вход рендерера)
  уже существующие CPU-fallback'ы: gfx/animate.rs, gfx/entity_batch.rs, gfx/flat.rs
```

### 3.1 Что переиспользуем, а не пишем заново

В коде уже есть CPU-ветки, написанные для wgpu-GL на старых GPU (Fermi, «OpenGL before 4.3»). Это главный рычаг:

| Узел | Файл | Что берём |
|---|---|---|
| Меши секций | [`mesher.rs`](../../crates/horizon-client-render/src/mesher.rs) | `SectionMesh.solid: Vec<TerrainVertex>` (24 байта/вершину), `Sides`, `translucent`, `visibility`, `sprite_layers`; **выключить** `mesher::packed::pack_quads(false)` — компактный layout читают шейдеры через storage-буфер |
| Сортировка прозрачных квадов | [`gfx/translucency.rs`](../../crates/horizon-client-render/src/gfx/translucency.rs) | чистый CPU-код `Sorter`/`SortState`, ванильный `VertexSorting` |
| Анимация спрайтов | [`gfx/animate.rs`](../../crates/horizon-client-render/src/gfx/animate.rs) | `Writer::Cpu` уже делает integer-mix и копирование mip-уровней; меняем только приёмник с `wgpu::Texture` на `glTexSubImage2D` |
| Позирование сущностей | [`gfx/entity_batch.rs`](../../crates/horizon-client-render/src/gfx/entity_batch.rs) | CPU-ветка «None without compute shaders» |
| Список draw'ов | `gfx.rs:4107`, `gfx/flat.rs` | уже есть «one draw per section reads `draws` on the CPU» при отсутствии indirect |
| HUD/меню | [`hud.rs`](../../crates/horizon-client-render/src/hud.rs) | `HudBatch { vertices, blends, patches, end_portal, … }` — готовые CPU-вершины и прямоугольники атласа |
| Атлас спрайтов | [`sprites.rs`](../../crates/horizon-client-render/src/sprites.rs), [`mipmap.rs`](../../crates/horizon-client-render/src/mipmap.rs) | все слои квадратные и одного размера, CPU mip-цепочки `sprite_mips`, `textureBudgetMiB` (сейчас `DEFAULT_TEXTURE_BUDGET = 256 MiB` — больше всей VRAM X1650!) |
| Кадр | `gfx.rs:656` `Frame<'a>` | камера, небо, туман, `HudBatch`, настройки — общий вход обоих рендереров |

### 3.2 Новый код

```
crates/horizon-client-render/src/gfx21/
  mod.rs          LegacyRenderer: совместимый публичный контракт (поля-заглушки + методы)
  context.rs      winit 0.30 + GLX/WGL, glow 0.17, dlopen libGL/opengl32
  caps.rs         GL_VERSION/GLSL/расширения/лимиты → выбор L1 / L0 / F0, лог в horizon.txt
  state.rs        кэш состояний: VBO/IBO, attribute pointers, texture units, blend/depth
  program.rs      компиляция GLSL 110/120, uniform-отражение, fallback при ошибке линковки
  shaders/        terrain.{vert,frag}, entity.*, hud.*, sky.*, line.*, blit.* (+ 110-варианты)
  atlas.rs        2D-атлас 2048² + gutters, UV-remap layer→rect, CPU mips, анимации
  mesh.rs         TerrainVertex → GlVertex (f32 атрибуты), VBO/IBO секций, origins на CPU в f64
  sections.rs     draw-list, frustum/cave culling, glMultiDrawElementsEXT / per-section draw
  passes.rs       последовательность: clear → sky → opaque/cutout → translucent → entities → HUD
  hud.rs          HudBatch → динамический VBO, Blend::* → GL-состояния
  screenshot.rs   FBO + glReadPixels → PNG
  diagnostics.rs  glGetError, стартовый отчёт, «Legacy mode: …» в horizon-ctl
```

`glow 0.17` и `libloading` уже есть в [Cargo.lock](../../Cargo.lock) как транзитивные зависимости `wgpu-hal`;
для создания контекста либо `glutin`/`glutin-winit` совместимой с `winit 0.30.13` версии, либо минимальный
GLX/WGL-враппер поверх `raw-window-handle` (тоже уже в lock'е). Новые зависимости проверяем по лицензиям
([CONTRIBUTING.md](../../CONTRIBUTING.md), §«Only original code») и добавляем в
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md).

### 3.3 Cargo-фичи и точки интеграции

* `wgpu`, `naga`, `pollster` становятся **optional** в `horizon-client-render` и `horizon-client`;
  `legacy-gl` собирает тот же бинарник без Vulkan/DX12/Metal.
* `pub type Renderer` переключается `#[cfg]`; реализован отдельный `game_legacy.rs` с точками `build_renderer`/`loading_renderer`.
* Две точки создания рендерера — `build_renderer` и `loading_renderer`
  ([`game.rs:20379`](../../crates/horizon-client/src/game.rs), `game.rs:20446`) — получают `#[cfg]`-ветки
  (в wgpu-ветке есть `renderer.hud.set_text_shader(&renderer.device, …)`, `set_cit_glints(&renderer.queue)` и т.п.).
* Новый вариант `GraphicsApi::LegacyGl` в [`graphics.rs:178`](../../crates/horizon-client-render/src/graphics.rs),
  ключ `graphicsApi:legacygl`, новый пункт в `GraphicsApi::choices()` на Linux/Windows и `--legacy-gl` в парсере
  опций `game.rs:675`. Применяется при следующем запуске, как и текущий `graphicsApi`.
* Недоступные методы контракта (`set_shader_pack`, `define_pack_macros`, `set_hdr`, `set_frame_generation`,
  `set_gpu_culling`, `queue_lods`, `upload_lods`, `set_pass_timing`, `gpu_memory_by_label`, …) — no-op/`None`/`0`
  с явным флагом `legacy: true`, чтобы UI показывал «недоступно», а `horizon-ctl` не падал.
* **Никаких захардкоженных строк в UI** ([AGENTS.md](../../AGENTS.md)): реализованы ключи
  `options.graphicsApi.*`, `options.legacy.unavailable`, `horizon.legacyGl.*` во всех языках, через `Text::tr`/`lang::get`.
* Паритетные харнессы не меняются: `options.txt` остаётся ванильным, новые ключи идут в `horizon-graphics.txt`
  ([docs/graphics.md](../graphics.md)).

---

## 4. Конвейер GL 2.1 подробно

### 4.1 Контекст и проверка возможностей

1. `winit 0.30.13` с `WINIT_UNIX_BACKEND=x11` (у `r300g` EGL/Wayland может отсутствовать); Windows — WGL,
   запрашиваем обычный контекст 2.1 (профили core/compatibility появляются только с GL 3.0).
2. `glGetString(GL_VERSION | GL_RENDERER | GL_SHADING_LANGUAGE_VERSION | GL_EXTENSIONS)` + `glGetIntegerv` всех
   лимитов из §1.2 пишем в `horizon.txt`. Если версии/расширений не хватает — внятная ошибка вместо чёрного экрана
   (аналог `NO_GPU_DRIVER` в `gfx.rs:1992`, но со списком конкретных отсутствующих расширений).
3. Обязательный минимум L0: `GL_ARB_vertex_buffer_object`, `GL_ARB_shading_language_100`,
   `GL_ARB_texture_non_power_of_two`, `GL_EXT_framebuffer_object` (или `GL_ARB_framebuffer_object`),
   `GL_ARB_multitexture`. Опции: `GL_EXT_multi_draw_arrays`, `GL_ARB_pixel_buffer_object`,
   `GL_ARB_shader_texture_lod`, `GL_EXT_texture_filter_anisotropic`, `GL_ARB_occlusion_query`, VAO.
4. `glow::Context::from_loader_function` для GL 2.0/2.1-вызовов; **не вызывать** GL3+-точки (`glGetStringi`,
   `glGenVertexArrays`, `glTexStorage*`, `glDraw*Indirect`) без проверки расширения.
5. VSync: `glXSwapIntervalSGI/MESA` или `wglSwapIntervalEXT`; present mode только `Fifo`/`Immediate`.

### 4.2 Шейдеры: рукописные GLSL 110/120

* Базовый вариант `#version 110` (GL 2.0), улучшенный `#version 120` (GL 2.1). Компилируем 120, при ошибке
  откатываемся на 110; в CI прогоняем оба.
* Спецификация — не автоконвертация: логика `terrain.wgsl` / `entity.wgsl` / `hud.wgsl` и ванильные
  `terrain.vsh/fsh`, `fog.glsl`, `sample_lightmap.glsl`, которые уже портированы в WGSL. Механически заменяем
  `texture()`→`texture2D()`, `@location`→`attribute`/`varying`, `out`→`gl_FragColor`, `textureSampleGrad`→
  `texture2DGradARB` только если есть `GL_ARB_shader_texture_lod`, иначе CPU-выбор mip/фильтр.
* Запрещено: `uint`/`int` как типы данных, битовые операции, `switch`, динамические циклы, `texelFetch`,
  `textureSize`, `round()` (в 1.10 его нет — `floor(x+0.5)`), массивы структур, `discard` до семплера,
  `gl_VertexID`, VTF. Uniform-массивы индексируются только константой или очень коротким циклом.
* Набор программ (≈12 пар, ≈1200 строк GLSL): terrain opaque/cutout/translucent, entity, HUD + glint,
  sky/ sunrise-fan / stars / clouds, weather/particles, lines/outline, panorama, blit для скриншота,
  упрощённый End-portal. В каждом ≤4 сэмплера и ≤32 `vec4` uniform'ов.
* Прогрев компиляции на загрузочном экране; кэш программ на весь process; при ошибке линковки — переход в F0.

### 4.3 Геометрия: от 24-байтного `TerrainVertex` к GL2-атрибутам

`TerrainVertex { pos: [f32;3], uv: [u16;2], layer: u16, light: u16, color: u32 }` нельзя отдать как есть:
нет целочисленных атрибутов и `flat`. Новый `GlVertex` (≤8 атрибутов, 32–40 байт):

| Атрибут | Тип | Источник |
|---|---|---|
| `a_position` | `vec3` (`GL_FLOAT`) | `pos`, уже camera-relative |
| `a_uv` | `vec2` (`GL_FLOAT`) | `uv / UV_STEPS` (`32768`) **+ смещение прямоугольника спрайта в 2D-атласе** |
| `a_color` | `vec4` (`GL_UNSIGNED_BYTE`, normalized) | `color` (RGBA8) |
| `a_light` | `vec2` (`GL_FLOAT`) | `light`: sky/block из 16x fixed point → UV лайтмапа |
| `a_fog` | `float` (или считаем из `a_position`) | расстояние для тумана |
| `a_flags` | `float` (опция) | cutout/opaqueness/emissive как float-код |

* `mesher::packed::pack_quads(false)`: распаковка компактных 24-байтных записей происходит на CPU в обычные
  4 вершины. Стоимость — рост VRAM и времени мешинга, поэтому лимиты в §6.
* Секционные origin'ы (сейчас их вычитает шейдер из integer-координат) считаем в **f64 на CPU** и подаём как
  `uniform vec3 u_section_origin` + `mat4 u_view_proj`. Большие мировые координаты в `float` не попадают.
* 6 индексов на квад в `GL_UNSIGNED_SHORT` (секция ≤65535 вершин; иначе отдельный VBO). `GL_UNSIGNED_INT`
  доступен с GL 1.2, но на R500 предпочтительнее 16-битные индексы и один VBO на секцию.
* Сущности: тот же конвертер для `mesher::Vertex` (`layer: u32`, `normal: [i8;4]`, `glint_uv`) → float-атрибуты;
  directional light и AO считаем по уже готовой CPU-ветке `entity_batch`.
* Линии/outline: `glLineWidth > 1` на Catalyst не гарантирован — используем уже существующее разложение ребра в
  4 вершины ([`gfx/core_lines.rs`](../../crates/horizon-client-render/src/gfx/core_lines.rs)), заменив
  `gl_VertexIndex` явным атрибутом «сторона».

### 4.4 Uniform'ы: дробим `Globals`

`Globals` в [`gfx.rs:335`](../../crates/horizon-client-render/src/gfx.rs) содержит ~200 `vec4` только в массивах
(`dynamic_lights[16]`, `colored_lights[32]`, `emissive_layers[128]`, `sunrise_fan[32]`, `hdr_windows[2]`), а предел
вершинного шейдера — 128 `vec4`. Разбиваем на независимые небольшие блоки и грузим только то, что нужно кадру:
`u_view_proj`, `u_fog`, `u_misc`, `u_time`, `u_lightmap`, `u_sun`, `u_sunrise_fan[32]`. Всё, что относится к HDR,
OIT, colored lights, emissive layers, visbuffer, в legacy-профиль не попадает вообще.

### 4.5 Атлас вместо `texture_2d_array`

Сейчас [`gfx.rs:1275`](../../crates/horizon-client-render/src/gfx.rs) делит спрайты на 8 массивов по 2048 слоёв.
GL2 такого нет. Решение:

1. CPU-паковщик `sprites::Sprites` (все слои квадратные, одного `size`) в **одну 2D-текстуру 2048×2048 RGBA8**
   с gutters 2–4 px. Для 16×16 спрайтов это ≈10 000 слотов, больше ванили 26.3; HD-паки принудительно
   downsample до 16/32 px (`sprites::finish`, `textureBudgetMiB`).
2. `layer → (u0,v0,u1,v1)` таблица; UV вершин пересчитывается один раз при загрузке меша. Для анимированных
   спрайтов — CPU-перезапись затронутых UV или `glTexSubImage2D` кадра в тот же слот (ветка `Writer::Cpu`).
3. Mip-цепочки строим на CPU из `assets.sprite_mips` (gutters обязательны, иначе bleeding), загружаем
   `glTexImage2D` по уровням; `glGenerateMipmapEXT` только как быстрый путь на L1.
4. `GL_CLAMP_TO_EDGE`, `GL_LINEAR_MIPMAP_LINEAR`, анизотропия ≤2–4. **Не включать** `GL_FRAMEBUFFER_SRGB`:
   ваниль смешивает в gamma-space, текущий `surface_format` тоже выбирает non-sRGB ([`gfx.rs:1720`](../../crates/horizon-client-render/src/gfx.rs)).
5. Отдельный GUI-атлас 1024² + `font::Patch` через `glTexSubImage2D`; глиф-кэш переиспользуется.

Бюджет VRAM (256 MiB-карта): атлас ≈21 MiB, GUI ≈5 MiB, лайтмап 1 KiB, depth 800×600×4 ≈2 MiB,
VBO/IBO ≤64 MiB, жёсткий потолок ≈128–160 MiB. `textureBudgetMiB:48`.

### 4.6 Глубина, смешивание, FBO

* Текущий reversed Z (`Depth32Float` + `GreaterEqual`) на GL2 делается одним из двух способов:
  **рекомендуется** обычный GL-диапазон: `camera::perspective(..., zero_to_one=false)`, `glClearDepth(1.0)`,
  `glDepthFunc(GL_LEQUAL)`; альтернатива — `glDepthRange(1.0, 0.0)` + `glClearDepth(0.0)` + `GL_GEQUAL`. Выбор
  закрепить прототипом PR5, потому что от знака Z зависят `ViewSet`, frustum culling и порядок проходов.
* Глубина: `GL_DEPTH_COMPONENT24` / `GL_DEPTH24_STENCIL8_EXT` renderbuffer в FBO
  (`GL_EXT_packed_depth_stencil`). `GL_DEPTH_COMPONENT32F` недоступен.
* FBO (`GL_EXT_framebuffer_object`) нужен для `--offscreen`, скриншотов и, опционально, одного post-прохода.
  Никаких float-аттачментов, MRT и MSAA.
* `glBlendFuncSeparate` для `Blend::Alpha/Additive/Invert/Vignette` и ванильного `CRUMBLING_BLEND`
  (`dst * src + src * dst`, alpha replace) из `gfx.rs:21`. Cutout — `glAlphaFunc(GL_GREATER, 0.1)` или `discard`
  в простом фрагментном шейдере. Прозрачные квады — строго back-to-front через `translucency::Sorter`,
  **без OIT** (нет атомики и float-блендинга).

### 4.7 Вызовы отрисовки

* Нет indirect и instancing. Один VBO на страницу/группу секций + `glMultiDrawElementsEXT`
  (`GL_EXT_multi_draw_arrays`) по списку `(first, count)`; если расширения нет — по одному
  `glDrawRangeElements` на секцию. Цель ≤200 вызовов на кадр при RD 4.
* Порядок: `clear → sky/stars/sunrise → opaque → cutout → translucent (sorted) → block entities →
  entities/particles → HUD → glint/outline`.
* CPU-culling: frustum + ванильный VisGraph (`caveCulling`), существующие `Sides`; GPU-culling, occlusion queries
  и асинхронный entity culling выключены.
* LOD/дальний террейн (`lod.rs`, `queue_lods`, `lodDistance`) в первой версии **выключены** — это compute +
  большие буферы, которых у R500 нет.

### 4.8 HUD и меню

`HudBatch` уже содержит `vertices: Vec<HudVertex>` и `blends: Vec<(Range<u32>, Blend)>`. Грузим в динамический VBO
(`glBufferData(GL_STREAM_DRAW)` + orphaning), один GLSL-программный HUD-шейдер с атласом и glint-текстурой.
`Blend::EndPortal` и `Blend::CitGlint` в первой версии либо упрощаются, либо отключаются с сообщением в чат/
настройках; `Blend::Depth` — изолированный depth + `glScissor`. Размытие меню (`menu_blur.wgsl`) и panorama-blur
отключены; кубическая панорама титульного экрана остаётся (`GL_ARB_texture_cube_map` есть).

### 4.9 То, что отключается полностью

| Фича | Файлы | Ключ | Состояние в legacy |
|---|---|---|---|
| Shader packs (WGSL API и `shaders/`-формат) | `gfx/native_pack.rs`, `gfx/pack_*`, `horizon-shader-api`, `horizon-shaders` | `pickedShaderPack` | недоступно, `set_shader_pack` no-op |
| HDR / HDR10 / scRGB | `gfx/hdr*.rs`, `hdr.wgsl`, `hdr_stats.wgsl` | `hdr` | forcibly false |
| TAA / FXAA / upscaling / FSR frame generation | `gfx/frame_gen/*`, `post_chain.wgsl` | `taa`, `fxaa`, `upscaling`, `frameGeneration` | forcibly off |
| OIT / visbuffer / flat GPU-пассы | `oit*.wgsl`, `gfx/visbuffer.rs`, `gfx/flat.rs` | — | off, обычные проходы |
| GPU culling / compute-скининг / GPU-анимация спрайтов | `gfx/gpu_culling.rs`, `gfx/entity_skin.rs`, `gfx/animate.rs` | — | CPU-ветки |
| Indirect multi-draw, translucent batching | `gfx/arena.rs`, `gfx/translucency.rs` | `batchTranslucent` | off, CPU draw-list |
| LOD / far terrain / BlueMap | `gfx/lod.rs`, `lod.rs`, `bluemap.rs` | `lodDistance` | 0 |
| Block-entity GPU caches | `block_entity_render.rs` | `blockEntityCache*` | off, обычный entity-буфер |
| Shadow cache, shadow passes | `gfx/*shadow*`, `gfx/pack_shadow.rs` | `shadowCache` | off |
| Timestamp queries / GPU census | `gfx/timing.rs`, `gfx/pack_timing.rs` | `overlay` | CPU-тайминги, `gpu_times = 0` |
| Capture/replay, splitscreen compositor, offscreen HDR | `gfx/capture.rs`, `gfx/compositor.rs` | — | недоступно |

`horizon-ctl` команды, которые читают GPU-статистику, должны отвечать `legacy: unsupported`, а не падать.

---

## 5. Профиль настроек по умолчанию

Новые/форсированные значения при первом запуске в legacy-режиме. `options.txt` сохраняет **ровно ванильные ключи**
(его сравнивает options-харнесс), всё остальное — `horizon-graphics.txt`:

```
# horizon-graphics.txt
graphicsApi:legacygl
lodDistance:0
farDetail:full            # при выключенном LOD не читается
caveCulling:true
entityCulling:off
leafCulling:off
batchTranslucent:false
workers:2
chunkUpdates:2
uploadBudgetKiB:2048
textureBudgetMiB:48
resolutionScale:100
fxaa:false
taa:false
upscaling:native
frameGeneration:off
hdr:false
ditherCutoutMips:false
animatedTextures:true      # CPU-ветка animate.rs
sky:true
fog:true
weather:true
batteryMode:true           # ≤60 FPS в мире, ≤30 в меню, половина worker'ов
shadowCache:false
pickedShaderPack:          # пусто
```

`options.txt`: `renderDistance:4`, `simulationDistance:4`, `mipmapLevels:2`, `maxFps:30`, `vsync:true`,
`particles:minimal`, `clouds:false` (или `fast`), `entityDistanceScaling:0.5`, `graphicsMode:fancy` только если
выдержит CPU. Мир желательно **предварительно сгенерировать** на современном хосте ([docs/pregeneration.md](../pregeneration.md)):
генерация 26.3 на двухъядерном CPU дороже отрисовки.

Пресеты (`Preset`) в legacy-режиме не должны перетирать форсированные ограничения: при выборе `Quality/Ultra`
недоступные строки серые и читаются «Off» — так уже сделано для `batchTranslucent` и HDR в
[docs/graphics.md](../graphics.md).

---

## 6. Порядок работ (каждый пункт ≈ один PR < ~500 строк, [AGENTS.md](../../AGENTS.md))

| # | PR | Содержимое | Gate |
|---|---|---|---|
| 0 | Дизайн и issue | этот документ, ссылка в [docs/README.md](../README.md); issue + `/claim` отложены | `git diff --check`, `tools/check_public_tree.py` |
| 1 | Капсы и инструмент | `tools/gl21_caps.py`, юнит-тесты, фикстура Catalyst 9.3, эталонный отчёт и шаблон; реальный дамп с X1650/`r300g` снимается на железе | `python3 -m unittest tests.test_gl21_caps -v`; `--check` на эталоне даёт `L1` |
| 2a | Аддитивный скелет | `gfx21/caps.rs`, `mod.rs`, `CONTRACT.md`, `pub mod gfx21;`, `tests/test_gl21_contract.py`; без правок `Cargo.toml` | Python-тесты контракта; дефолтная сборка не изменена |
| 2b | Развязка зависимостей | `wgpu`/`naga`/`pollster` → optional за `default = ["modern-gl"]`, фича `legacy-gl` в трёх крейтах, `pub type Renderer`/`Frame` под `#[cfg]`; полный `#[cfg]`-разбор сделан в PR 8 | gate `cargo check -p horizon-client --features legacy-gl --no-default-features` и `cargo tree` без modern-крейтов в [нашем workflow](../../.github/workflows/arena-legacy-gl.yml); обычная сборка без изменений |
| 3 | Контекст | stage 1: `context.rs`, `state.rs`, `smoke.rs` (headless `gl21_clear` команды без GL-вызовов); stage 2: `examples/gl21_clear.rs` с реальным GLX/WGL-контекстом, `glGet*`, glow-загрузчиком и треугольником на экране | юнит-тесты `gfx21` и два Xvfb-кадра в нашем workflow (`GL_NO_ERROR`); затем запуск на X1650 |
| 4 | Шейдеры | готово: 24 файла 120/110 для smoke/terrain/entity/sky/HUD/line, `validate_all()`, `program_for()`/`all_programs()`; пример линкует весь набор в обоих кадрах | `all programs linked: OK` дважды; `validate_all()` и Python-тест 24 источников |
| 5 | Атлас и конвертер | готово: `atlas.rs`, `mesh.rs`; независимые тесты `TerrainVertex → GlVertex` (`UV_STEPS`, light 16x, RGBA8, 36 байт), mip-gutters, ≤4 сэмплера | 75 изолированных Rust-тестов + `clippy -D warnings`; скриншот одного спрайта против wgpu-эталона остаётся вне этого продолжения |
| 6 | Террейн | готово: `sections.rs`, `passes.rs`, `frame.rs`; opaque/cutout/translucent, `glMultiDrawElementsEXT`, туман, лайтмап, CPU golden [`PNG`](../../benchmarks/graphics/legacy-gl-scene.png), ≤200 draw calls | `tools/legacy_gl_golden.py --check` побайтно сравнивает кадр с независимым Python-рендером; wgpu-diff и реальная X1650 остаются вне этого продолжения |
| 7 | Сущности, небо, HUD | готово аддитивно: `blend.rs`, `entities.rs`, `sky.rs`, `hud.rs`, `lines.rs`, `render.rs`, `FullFramePlan`, полная таблица `BlendMode`/`SortState` | независимые тесты пикселей title/inventory/chat/hotbar, 16 draw calls, все шесть программ |
| 8 | Полная интеграция | `facade.rs` плюс `#[cfg]`-разбор, `game_legacy.rs`/`legacy_gl.rs`, `GraphicsApi::LegacyGl`, UI/локализация, реальный GL2-кадр, `legacy-gl` без `modern-gl` | `size_of::<Renderer>() == 0`, контрактные имена покрыты, `cargo tree` без wgpu/naga/pollster/`modern-gl`, два игровых Xvfb-кадра |
| 9 | CI и диагностика | `diagnostics.rs`, [`tools/legacy_gl_smoke.py`](../../tools/legacy_gl_smoke.py), stats/startup fixtures, debug `glGetError`, `tests/test_gl21_integration.py`, `--check` и игровые кадры в workflow; **без wgpu-diff** | `legacy-gl smoke: OK`, изолированные Rust-тесты + полные Python-тесты, 14 шагов своего workflow |
| 10 | Распространение | готово, затем обновлено под полную интеграцию: [README.md](../../README.md), [docs/native-client.md](../native-client.md), нотисы, план, контракт и CI; roadmap-пункт не заводился без одобрения | 0 битых ссылок, `YAML_OK ... steps 14`; реальная X1650/30 минут не измерялись |

Порядок зависимостей: `1 → 2a → 2b → 3 → {4,5} → 6 → 7 → 8 → 9 → 10`. PR 4 и 5 можно делать параллельно. После аддитивных PR 3–10 по явному запросу пользователя выполнен полный PR 8: `#[cfg]`-разбор `gfx`/`game.rs`, подключение реального GL2-рендерера к игре, удаление `modern-gl` из `legacy-gl`, UI/локализация и собственный CI с игровыми кадрами. wgpu-паритет и реальная X1650 остаются вне объёма и не измерены.

---

## 7. Тестирование и критерии приёмки

### 7.1 CI без реального железа

Mesa позволяет притвориться GL 2.1 ([envvars](https://mesa-docs.readthedocs.io/en/latest/envvars.html)):

```sh
export LIBGL_ALWAYS_SOFTWARE=1
export MESA_GL_VERSION_OVERRIDE=2.1
export MESA_GLSL_VERSION_OVERRIDE=120
export MESA_EXTENSION_MAX_YEAR=2006          # спрятать современные расширения
export MESA_EXTENSION_OVERRIDE='-GL_ARB_uniform_buffer_object -GL_EXT_texture_array -GL_ARB_instanced_arrays'
xvfb-run -s '-screen 0 800x600x24' \
  cargo run --profile fast -p horizon-client --no-default-features --features legacy-gl -- --legacy-gl
```

Отдельный прогон с `MESA_GL_VERSION_OVERRIDE=2.0` проверяет профиль L0. Mesa 26.0.8 на `ubuntu-latest`
не отдаёт `GL_SHADING_LANGUAGE_VERSION=1.10` при `MESA_GLSL_VERSION_OVERRIDE=110` (падение внутри
`shading_language_version()`), поэтому GLSL-override оставляем `120`, а диалект 110 задаём примеру явно через
`HORIZON_GLSL_DIALECT=110`. CI запускает и отдельный пример с реальным контекстом, и полноценный игровой цикл:

```sh
for spec in "2.1 120 120" "2.0 120 110"; do
  set -- $spec
  MESA_GL_VERSION_OVERRIDE="$1" MESA_GLSL_VERSION_OVERRIDE="$2" HORIZON_GLSL_DIALECT="$3" \
    xvfb-run -a -s '-screen 0 800x600x24' \
    cargo run -p horizon-client-render --example gl21_clear --no-default-features --features legacy-gl
    xvfb-run -a -s '-screen 0 800x600x24' \
    cargo run -p horizon-client --no-default-features --features legacy-gl -- --legacy-gl --frames 1
done
```

Пример печатает `GL_VERSION`/`GLSL_VERSION`, выбирает `Glsl120`/`Glsl110`, линкует все шесть программ из
`gfx21/shaders/` (`all programs linked: OK`), рисует smoke-треугольник и завершает кадр с `GL_NO_ERROR`.

Тот же отчёт до запуска игры снимает инструмент PR 1:

```sh
LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1 \
MESA_GLSL_VERSION_OVERRIDE=120 MESA_EXTENSION_MAX_YEAR=2006 \
python3 tools/gl21_caps.py --label ci-gl21 --check --no-write --stdout
```

Важно: override меняет **отчёт**, а не реальные возможности драйвера, поэтому тест дополняется статической проверкой:
ни один GL3+-вызов не должен встречаться в исходниках `gfx21` (grep-тест), а `caps.rs` в CI работает в
«strict»-режиме и считает запрещённые расширения отсутствующими.

Эти Python-проверки (включая `tests.test_gl21_smoke` и `tools/legacy_gl_smoke.py --check --stdout`), gate PR 2b, изолированные тесты `gfx21`, golden-кадр, PR 9 startup report и оба GL-кадра под Xvfb выполняет и проходит
собственный workflow ветки [`arena-legacy-gl.yml`](../../.github/workflows/arena-legacy-gl.yml): на ранере GitHub доступны
crates.io и Rust 1.99 (≥1.95), поэтому там запускаются `cargo check -p horizon-client --features legacy-gl
--no-default-features`, `cargo test -p horizon-client-render --lib gfx21 --no-default-features --features legacy-gl`, `rustfmt`,
`clippy`, `GFX21_GOLDEN_OUT=... cargo test` + `tools/legacy_gl_golden.py --check` и `cargo run --example gl21_clear`
с `MESA_GL_VERSION_OVERRIDE=2.1/2.0` и `HORIZON_GLSL_DIALECT=120/110`. Чужие actions, PR и workflow не просматриваются и не меняются.

### 7.2 Проверки

* Готово: юнит-тесты атласа, конвертера вершин, секций, multi-draw, пикселей golden-кадра, таблицы
  `BlendMode`/`SortState`, CPU-posing сущностей, неба, HUD и линий — с жёстко заданными ожидаемыми
  значениями, а не вычисленными тем же кодом ([AGENTS.md](../../AGENTS.md), «Write tests with signal»).
* `gfx21::program::validate_all()` и Python-тест `test_shader_sources_stay_in_the_110_120_dialect`
  проверяют все 24 источника `gfx21/shaders/` и запрещают GLSL 3+/GL3-токены; пример линкует все шесть
  программ обоих диалектов под Mesa override (`all programs linked: OK`).
* PR 6 добавил детерминированный CPU golden-кадр 80×60 ([`PNG`](../../benchmarks/graphics/legacy-gl-scene.png),
  [`tools/legacy_gl_golden.py`](../../tools/legacy_gl_golden.py)): Rust-растеризатор и независимый
  Python-рендер побайтно совпадают. PR 9 добавил [`stats.json`](../../benchmarks/graphics/legacy-gl-scene.stats.json)
  и [`startup report`](../../benchmarks/graphics/legacy-gl-startup-report.txt) без GL-контекста. Это **не**
  паритет с wgpu: те же 4 сцены/камеры из `tools/graphics_bench.py`
  ([docs/graphics.md](../graphics.md)), diff PNG по доле пикселей и средней ошибке остаются вне этого продолжения.
* Все существующие паритетные харнессы остаются зелёными: симуляция, `options.txt`, сохранения и сеть не меняются.
* `python3 tools/pr_ready.py` перед каждым PR; `cargo test -p horizon-client-render --lib gfx21 --no-default-features --features legacy-gl`
  в [нашем workflow](../../.github/workflows/arena-legacy-gl.yml) —modern-стек уже снят, `cargo tree` это проверяет.
* `ldd target/fast/horizon-client | grep -E 'vulkan|EGL'` — пусто; присутствуют `libGL.so.1`, `libX11.so.6`.

### 7.3 Приёмка на реальной X1650

1. `glxinfo -B`: `OpenGL 2.1 Mesa …`, `GL_RENDERER: Radeon X1650`, GLSL 1.20, `r300`/`r300g`.
2. Стартовый лог содержит версию, лимиты, выбранный профиль L1/L0 и список отключённых фич.
3. Титульный экран → мир (желательно прегенерённый) → 30 минут игры: ни одного `GL_ERROR`, ни одного
   device-lost/краша, утечки VRAM нет.
4. 800×600, RD 4, `textureBudgetMiB:48`: стабильные ~10–25 FPS (цель — играбельность, не рекорд), ≤200 draw calls,
   ≤160 MiB VRAM, CPU ≈2 потока мешинга.
5. Скриншот, инвентарь, чат, хотбар, туман, смена времени суток, анимация воды/портала работают.
6. При откате GLSL-компилятора включается F0 и игра всё равно показывает мир (не чёрный экран).

В этом аддитивном продолжении перечисленные проверки на реальной X1650 не выполнялись: нет карты, `r300g`, `glxinfo -B -l`, 800×600 таймингов и 30-минутного прогона. Фикстуры и отчёты явно помечены `NOT measured in this sandbox`.

---

## 8. Риски

| Риск | Вероятность | Митигация |
|---|---|---|
| GLSL-компилятор Catalyst/r300 не переваривает даже 110-шейдеры | высокая | профиль F0 на ARB-программах/fixed-function; компиляция с fallback'ом в PR4 |
| `Globals` не влезает в 128 `vec4` | высокая | дробление uniform'ов (§4.4), отказ от extras в legacy |
| Перевёрнутый Z даёт артефакты/инвертирует culling | средняя | прототип в PR5, явный выбор `zero_to_one=false` до интеграции |
| VRAM 256 MiB исчерпан атласом/мешами | высокая | 2048² атлас, `textureBudgetMiB:48`, RD ≤6, жёсткий потолок VBO, отключённый LOD |
| CPU-мешинг и сортировка съедают двухъядерный CPU | высокая | `workers:2`, `chunkUpdates:2`, прегенерация мира, `batteryMode` |
| wgpu всё равно линкуется/требует `libvulkan.so.1` | средняя | optional-зависимости в **обоих** crates, проверка `ldd` в CI |
| glibc/инструкции: бинарь с современного хоста не стартует | средняя | сборка в контейнере со старой glibc или musl + `dlopen(libGL)`; `RUSTFLAGS='-C target-cpu=x86-64'`, проверить POPCNT/AVX/SSE4.2 в зависимостях (в [Cargo.toml](../../Cargo.toml) уже есть патч mimalloc для Core 2) |
| Лаунчер GPUI сам требует Vulkan/DX12 | высокая | отдельный архив с бинарём + подготовленными `.local/horizon-client/26.3`; `.local/` не коммитить ([AGENTS.md](../../AGENTS.md)); авторизацию/профиль перенести с современного хоста |
| Изменения настроек ломают options-харнесс | средняя | только `horizon-graphics.txt`, `options.txt` ванильный, тесты PR8 |
| Native mods / shader-pack API ожидают wgpu-объекты | средняя | `#[cfg]`-заглушки, явное «unsupported in legacy mode», без падения |
| Драйвер не отдаёт `GL_EXT_framebuffer_object` | низкая | F0/FBO-less путь: рисуем прямо в окно, скриншот через `glReadPixels` back buffer |

---

## 9. Что **не** входит в первую версию

Шейдер-паки (оба формата), HDR, TAA/FXAA/FSR/апскейл, OIT, visbuffer, frame generation, GPU-culling,
дальний LOD/BlueMap, тени шейдер-паков, splitscreen/compositor, capture/replay, timestamp-профилирование GPU,
HDR-скриншоты, CIT-glint и точный End-portal бленд, HD-ресурс-паки выше 32 px/спрайт, macOS, Wayland/EGL,
Windows 10/11, 32-битные сборки, серверные рендер-фичи (их нет и сейчас).

После того как L1 стабилен, отдельным этапом можно вернуть часть post-эффектов одним RGBA8-FBO проходом
(простое FXAA/виньетка), но это не условие запуска.

---

## 10. Источники

* Дамп возможностей `Radeon X1600/X1650 Series`, Catalyst 9.3: <https://vgamuseum.info/images/doc/ati/x1650.txt>
* Драйвер `r300` для R300–R500 и OpenGL 2.1 в Mesa: <https://wiki.archlinux.org/title/OpenGL>,
  <https://mesa-docs.readthedocs.io/_/downloads/en/latest/pdf/>
* Переменные Mesa для эмуляции версии в CI: <https://mesa-docs.readthedocs.io/en/latest/envvars.html>
* Внутри репозитория: [README.md](../../README.md), [docs/graphics.md](../graphics.md),
  [docs/architecture.md](../architecture.md), [AGENTS.md](../../AGENTS.md),
  [`crates/horizon-client-render/src/gfx.rs`](../../crates/horizon-client-render/src/gfx.rs),
  [`third_party/wgpu-hal/src/gles/adapter.rs`](../../third_party/wgpu-hal/src/gles/adapter.rs),
  [`third_party/naga/src/back/glsl/mod.rs`](../../third_party/naga/src/back/glsl/mod.rs).
