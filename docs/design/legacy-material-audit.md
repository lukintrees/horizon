# Аудит material codes и Frame/HUD флагов legacy GL2 (M01a)

> 🤖 Статус: **draft, ждёт review интегратора**. Не принято. Каталог подтверждён чтением исходников; код не запускался.

- **Task-ID:** M01a (условный M01 из [плана запуска](legacy-gl-launch-plan.md))
- **Variant:** A — аудит и воспроизводимая статическая проверка; без новых passes и без изменения dispatcher
- **Baseline SHA:** `453966cb6253adf848deb58165eba354693e7acc` (= голова `arena/a7e96628-horizon` на момент работы). Все `файл:строка` ниже относятся к этому SHA.
- **Central files changed:** нет. Facade, contracts, `game.rs`, `entry.rs`, `legacy_gfx.rs`, `legacy_gfx/frame.rs`, dispatcher, `Cargo.*`, workflows и policy scripts не изменены.

## 0. Ограничения этого аудита

- В песочнице **нет Rust toolchain** (ни `cargo`, ни `rustc`). Поведение Rust-кода выведено из исходников, а не из запусков тестов. Существующие Rust-тесты, на которые ссылается каталог, **не запускались**.
- Ресурсы Mojang отсутствуют. Поэтому нельзя проверить, какие реальные ассеты дают нестандартные UV, и существует ли `minecraft:end_of_frame` в ванильных данных.
- Win7 и Radeon X1650 не проверялись; ничего из этого документа не является qualification.
- Тесты «без синтетических quads»: вывод о поддержке каждого кода сделан по producer'у в `Game` и по правилам `entity::runs_view`, а не по тестовым квадам.

## 1. Воспроизведение

```sh
python3 tools/legacy_material_audit.py                      # 76 статических проверок -> target/legacy-material-audit/report.json
python3 -m unittest discover -s tests -p "test_legacy_material_audit.py"
```

`tools/legacy_material_audit.py` читает исходники и сверяет их с
[`tests/fixtures/legacy_material_audit/expectations.json`](../../tests/fixtures/legacy_material_audit/expectations.json).
Ожидания записаны вручную. Инструмент проверяет:

- какие коды принимает `entity::runs_view` (разбор match-arms `crates/horizon-render-api/src/entity.rs`), в том числе для additive-вида и glint;
- список блокеров `legacy_gfx/frame.rs::unsupported()`;
- значения констант overlay;
- наличие каждого producer-сниппета и каждого «дефолт включён» флага;
- отсутствие тестов для перечисленных кодов и отсутствие подключения `TerrainHudRenderer` к `Game`.

Проверка доказывает соответствие каталога исходникам на этом SHA. Она не доказывает корректность рендера.

## 2. Классы

| Класс | Смысл |
|---|---|
| **P0-B2** | Блокер обычного gameplay первого мира. Без решения B2 объявлять нельзя. Молчаливый пропуск геометрии запрещён (план, M01). |
| **P0-профиль** | Решается профилем legacy (F01/U01): явное значение флага и capability-ошибка или лог. Renderer-работы не нужно, но без решения кадр отклоняется. |
| **P0-wiring** | Код в legacy-модуле есть, но `Game` его не вызывает. |
| **Optional** | Визуал вне первого мира или выключенный по умолчанию. Может остаться отклонённым, если профиль его выключает. |
| **Supported** | Путь есть в исходниках legacy, с указанной policy. Не означает проверку на Win7. |

## 3. Frame-флаги, которые `Game` выставляет (`gfx::Frame`, `crates/horizon-client-render/src/gfx.rs:656`)

Колонка «в обычной игре» показывает, срабатывает ли флаг без опций. «Legacy» — что делает `legacy_gfx/frame.rs`.

| Флаг | Producer (файл:строка) | В обычной игре | Legacy | Класс | Тест legacy |
|---|---|---|---|---|---|
| `outline` (контур блока) | `game.rs:15664` (`let outline = if !self.ui.hide_gui && ...`, из `pick()`) | **да**, при взгляде на любой блок, в том числе при открытом инвентаре | `unsupported` «lines/outlines» (`frame.rs:354`) | **P0-B2** (V01) | нет |
| `debug_lines` (удочка) | `game.rs:15653`, `game/fishing.rs:966` `fishing_line` | **да**, пока заброшена удочка | «lines/outlines» | **P0-B2** (V01) | нет |
| `debug_lines_on_top`, `debug_crosshair` | `game.rs:15653`, `game.rs:15655` (F3 и отладочные gizmo) | нет, только в debug | «lines/outlines» | Optional (P2) | нет |
| `clouds` | `game.rs:15957` при `settings.clouds` = `"true"` по умолчанию (`settings.rs:230`) | **да**, overworld по умолчанию | «clouds» | **P0-профиль** (явный clouds-off до V04) | нет |
| `chunk_fade` | `game.rs:16044` ← `settings.rs:239` `chunk_fade: 0.75` | **да**, каждый кадр с миром | «chunk fade» | **P0-профиль** (legacy задаёт 0 явно или реализуется) | **да** (`frame.rs` тест) |
| `post_effects` с `minecraft:end_of_frame` | `game.rs:16128` `requested_post_effects()` добавляет имя всегда, если есть игрок и нет title | **да**, каждый кадр с миром, даже без реальной цепочки | «post effects/menu blur» по `!effects.is_empty()` | **P0-профиль/семантика**: legacy должен проверить наличие цепочки в активном root (отсутствие = пропуск, как в `post_chain.rs`, где `chains` = `None`) | нет |
| `menu_blur` | `game.rs:16139`, радиус по умолчанию 5 (`settings.rs:247`) | **да**, пауза, диалоги, экраны настроек | «post effects/menu blur» | **P0-B2** для меню (U01, статичный фон) | **да** (`frame.rs` тест) |
| `title` | `game.rs:15722` `shows_panorama()` (title, `remote_loading`, паки с панорамой) | **да** на стартовом экране и при ожидании сервера | «title panorama» | **P0-B2** (U01 static background) | **да** |
| `glowing` | `game.rs:15889` ← `game/glowing.rs` | только при эффекте свечения/командах | «glowing silhouettes» | P1 optional (V03) | нет |
| `skin` (GPU-поза) | `game.rs:15978` `skin.pending().then_some(&skin)` | только модерн-рендер (`skins_entities`, `gfx.rs:5888`) | «GPU-posed entities» | **P0-wiring** (E01: legacy не создаёт skin store) | нет (только `skin: None` в fixture) |
| `border` | `game.rs:15869` вблизи границы мира | нет, только у границы | «world border» | Optional (V05) | нет |
| `colored_lights`, `emissive_layers` | `game.rs:15623`, `15628` (нативные моды, паки) | нет по умолчанию | «dynamic/colored lights», «emissive layers» | Optional | нет |
| `dynamic_lights` | `game/dynamic_lights.rs` ← `extras.rs:166` `dynamic_lights: false` | нет по умолчанию | «dynamic/colored lights» | Optional | нет |
| `water_plane` | `game.rs:15912` только если `wants_water_plane()` (модерн-паки) | нет | «water mirror» | Optional, не для legacy | нет |
| `crumbling_sprite` | `game.rs:16003` (из `block/destroy_stage_0`) | при ломании | не блокирует напрямую, но нужен для V02 | **P0-B2** (см. §4) | нет |

Поля, которые legacy игнорирует или которые не требуют пасса: `entity_spans`, `entity_motion`, `pack`, `shader_game_time`, `shadow_only` (только нативный паки-shadow), `shadow_scene_revision`, `static_block_entity_stats`. Для `static_block_entities` есть отдельная обработка в `Prepared::new` (`legacy_gfx/frame.rs`).

## 4. Overlay-коды (`Vertex::layer & 0xffff`)

Правила приёма (`crates/horizon-render-api/src/entity.rs`, `runs_view`, строки ~222–266) и producer'ы. Колонка «тест» — существующая проверка в исходниках; «нет» значит, что в legacy-тестах этого кода нет.

| Код | Producer (файл:строка) | Что это | Legacy | Нужная policy | Тест | Класс |
|---|---|---|---|---|---|---|
| **0** | `item_render.rs` `(false,false) => 0`; тела мобов, частицы без translucent | обычная модель, опак. | принят, Body | depth write, cull по normal | да (`entity.rs`) | Supported |
| **1** | `game/dragon.rs:3015` `u32::from(d.hurt_time > 0)`, мобы при уроне | hurt-тинт | принят, `Hurt` | красный overlay 178/255 перед светом | косвенно (`entity.rs` `two_sided_normals_overlay...`) | Supported |
| **5** | белая вспышка (`overlay.rs`) | white overlay | принят, `White(15)` (`entity.rs:235`) | альфа по `White(u)` | **нет прямого** | Supported (без теста) |
| **16–31** | `mobs/render.rs:1310` `overlay::creeper_progress` (`overlay.rs:39`) — взрыв крипера | белая вспышка по прогрессу | принят, `White(code-16)` (`entity.rs:236`) | по `White(u)` | **нет прямого** (`overlay.rs:96` — modern-тест прогресса) | Supported (без теста) |
| **2** | `gfx/lightning.rs:87` `2 \| VERY_BRIGHT<<16`; `gfx/end_sky.rs:72` | аддитивные молнии и End-небо | только additive-вид; обычный вид отклоняет | SRC_ALPHA/ONE, без depth write, свет 1 | да (`legacy_gfx.rs` bridge test) | Supported |
| **3** | additive-вид (код допускается в guard) | аддитивный | additive-вид | как 2 | да | Supported |
| **7** | `item_render.rs` `(true,false) => 7` (фольга на непрозрачном предмете) | glint | **принят только при `Glint` опции**, иначе отказ | fragment glint по atlas UV, сила glint | `entity.rs` `glint_variants_preserve...`; native `backend_entity_glint` | **P0-wiring** |
| **8** | hurt + glint (`entity.rs` `1 \| 8`, glint) | hurt + glint | как 7 | как 7 | `entity.rs` `glint_variants...` | **P0-wiring** |
| **33** | `item_render.rs` `(true,true) => TRANSLUCENT_GLINT_ITEM_OVERLAY` | прозрачный предмет + glint | как 7 | как 7 без depth-write | `entity.rs` `glint_variants...` | **P0-wiring** |
| **36** | `mobs/render.rs:23` `ARMOR_GLINT` для зачарованной брони | glint брони | как 7, `armor_uv` | как 7, отдельный armor UV | `entity.rs` `glint_variants...` | **P0-wiring** |
| **13** | `game/boats.rs:1520` `WATER_MASK_OVERLAY` | маска воды лодки | принят, WaterMask, без RGBA | write depth, no colour | да | Supported |
| **14** | `shadows.rs:214` `SHADOW_OVERLAY` | тень сущности | принят, Shadow | тёмная полупрозрачная, cull back, без depth write | да | Supported |
| **15** | `particles.rs:4543` `TRANSLUCENT_PARTICLE_OVERLAY` | полупрозрачная частица | принят, Particle | blend, без depth write | да (`entity.rs` cull test) | Supported |
| **32** | `item_render.rs` `(false,true) => TRANSLUCENT_ITEM_OVERLAY` | прозрачный предмет | принят | blend, без depth write | да | Supported |
| **35** | `game/text_displays.rs:380` `TEXT_POLYGON_OFFSET_OVERLAY` | текст display | принят, Text (polygon offset) | depth bias к камере | да | Supported |
| **37** | `game/dragon.rs:4195` `DRAGON_RAYS_OVERLAY` (только additive) | лучи дракона | **только additive**; в обычном виде отказ | ADD, cull back, depth write | да | Supported |
| **38** | `game/dragon.rs:3013` `DISSOLVE_OVERLAY` (умирающий дракон, End) | dissolve-маска | **отказ** | alpha-discard по маске, atlas UV в `glint_uv` | отказ — да (`entity.rs` тест), поддержки нет | Optional (End, P1) |
| **34** | `mobs/render.rs:2270` `ENERGY_SWIRL_OVERLAY` (заряженный крипер, броня иссушителя) | энерго-вихрь | **отказ** | отдельный blend, UV-скролл | отказ — да (`entity.rs` тест), поддержки нет | **P0-B2** (редкий, но обычный; не пропускать молча) |
| **11** | `breaking.rs:82` `vertex.layer = 11;` и `breaking.rs:48` (`translucent ? 12 : 11`), вызовы из `game.rs:11355` (`breaking::geometry`) и `block_entity_render.rs:2213` (`entity_geometry`) | **трещины ломания блока**, выставляются в `entities` | **отказ**, нет тестов | см. ниже | **нет** | **P0-B2** (V02) |
| **12** | `breaking.rs:48` (полупрозрачный блок с трещиной) | трещины полупрозрачного блока | **отказ**, нет тестов | как 11 | **нет** | **P0-B2** (V02) |
| **4** | `clouds.rs:224` `layer: 4` | облака | отказ (на уровне кадра «clouds») | отдельный модуль V04 | нет | P0-профиль / Optional |
| **6** | `block_entity_render.rs:3148` (`emit`, end portal/gateway) | End portal/gateway | отказ | portal shader, не квад | нет | Optional (End, V03) |
| **9, 10, 17–31 вне диапазона, 38+** | не найдено в producer'ах | — | отказ по `_` arm | — | 9 — да | — |

Регрессия здесь, а не в каталоге: отказ происходит до очистки кадра и отменяет **весь** поток сущностей кадра (`render_geometry` возвращает ошибку). Один крипер со вспышкой или одна трещина ломания приводят к потере кадра сущностей. Поэтому такие коды — P0, а не «отдельная визуальная задача».

### 4.1. Трещины ломания (11/12) — policy

Modern-эталон (`gfx.rs:21` `CRUMBLING_BLEND`, `gfx.rs:3332` `crumbling_pipeline`): blend `src=Dst, dst=Src` (то есть `2·src·dst`), **depth write выключен**, depth test `GreaterEqual` с bias 10 (reversed Z), cull back. Координаты: origin спрайта в light-байтах (`breaking::stage_light`), размер в `crumbling_sprite.zw`, UV проецируется и повторяется внутри спрайта (`gfx/crumbling_tests.rs`, `crack_samples_stay_aligned_between_stages`). Для legacy нужно зеркало: `glBlendFunc(GL_DST_COLOR, GL_SRC_COLOR)`, conventional depth (LEQUAL), depth write off, cull back, polygon offset. **Это перенос policy из modern-эталона, не сверка с ванилой.** Модерн-тест — единственный тест этого поведения, legacy-readback отсутствует.

## 5. Поддержанные terrain/HUD/небо/погода

| Путь | Producer | Legacy | Тест | Класс |
|---|---|---|---|---|
| Terrain solid / translucent | `mesher.rs` → `render_bridge.rs:69–70` → `terrain::runs` (`terrain.rs`) | `section_runs` с `translucent` флагом; отказы `MissingSprite`, `MixedLayers`, `RepeatingUv` | да (`render_bridge` тест, probe `backend_terrain_atlas_contract`) | Supported, **риск** ниже |
| Terrain cutout | `terrain.rs` `layer & 0x7fff`; `OPAQUE_LAYER` в `mesher.rs` | pass Cutout по sprite | да (native) | Supported |
| HUD / GUI / текст | `hud.rs`, `gui_bridge.rs` | `TerrainHudRenderer`, страницы | да (native, 9 GPU fixtures) | Supported, **не подключён к Game** |
| Небо, звёзды, celestial, dark disc, End sky | поля `Frame` (`sky`, `celestial`, `stars`, `skybox`, `end_sky`), сборка в `game.rs` | `legacy_gfx/sky.rs` (`skybox`, `end_sky`, `celestial`, `stars`) | да (`sky.rs` тесты) | Supported |
| Погода (дождь/снег) | `weather.rs` | `legacy_gfx/weather.rs` | да (`weather.rs` тесты) | Supported |
| Screen-эффекты (`ScreenQuad`) | `crates/horizon-client-render/src/screen_effects.rs` | `legacy_gfx/screen.rs` | да (`screen.rs` тесты) | Supported (перечень эффектов не сверен поштучно) |

**Риск терраина.** `terrain.rs` отказывает, если UV вне `[0,1]` (`RepeatingUv`). Обычный mesher кодирует sprite-local UV (`TerrainVertex::uv`, `mesher.rs:116`). Редкие модели с UV за пределами 0–16 могут вызвать отказ. Это **не проверено на реальных моделях**, так как ассетов нет. LOD-путь с repeat UV (`TerrainVertex::tiled`) в `SectionMesh` не входит и относится к B4.

**Проводка к Game.** `TerrainHudRenderer` и `set_glint_textures` (`legacy_gfx.rs:102`, `:167`) не вызываются из `crates/horizon-client/src` (проверено `absent`-проверкой). Поэтому `legacy_gfx` сейчас не может обслужить ни один кадр `Game`.

## 6. P0-список для первого мира (B2)

Порядок — по зависимостям плана; владелец указан по регламенту.

1. **Контур блока (`outline`)** — каждый кадр при взгляде на блок. Нужен CPU/screen-space пасс `V01`. Для интегратора: добавить вид/пасс линий в dispatcher (общая зона). Без этого любой кадр с блоком отклоняется.
2. **Трещины ломания (11/12)** — `V02`, policy из §4.1. Для интегратора: слот пасса и pass-id, решение по порядку с translucent terrain.
3. **Удочка (`debug_lines`)** — тот же путь `V01`.
4. **Профиль: `chunk_fade`, `clouds`, `end_of_frame`, `menu_blur`, `title`** — F01/U01. Внутри legacy-профиля:
   - `chunk_fade = 0` явно, с логом;
   - облака выключены (до V04), с логом;
   - `end_of_frame` включается только при наличии цепочки в активном root; иначе запрос не отправляется (семантика модерн-пути);
   - блюр меню выключен (статичный фон), пауза и инвентарь показывают ту же геометрию без блюра;
   - заставка (`title`) — статичный фон.
   Все эти решения меняют состав флагов, поэтому их вносит интегратор (`game.rs` и настройки профиля).
5. **Зачарованные предметы (7/8/33/36)** — нужна проводка `set_glint_textures` из загрузчика ресурсов в `Game` (A02/E01). Без неё любой видимый зачарованный предмет отклоняет поток сущностей. Тесты native glint есть, Game-проводки нет.
6. **Энерго-вихрь (34)** — заряженный крипер и броня иссушителя. Редкий, но обычный в выживании. Варианты: реализовать (V03), либо явно выключить в профиле с объяснением в UI. Молчаливый пропуск запрещён.
7. **Скин GPU (`skin`)** — legacy не должен получать `Some`. Сейчас это выполняется, потому что `skins_entities()` требует модерн-рендер; при подключении legacy проверить (E01).

## 7. Optional (не блокируют первый мир, но должны быть явно выключены в профиле)

- **Dissolve (38)** — умирающий дракон в End. P1, B3/B4.
- **End portal/gateway (6)** — End. V03, B4.
- **Облака (4)** — V04. До реализации профиль выключает их (см. P0 п. 4).
- **Свечение (glowing)** — при эффекте свечения кадр отклоняется. P1, решается V03 либо явным отказом в UI.
- **Граница мира (border)** — только вблизи границы. V05.
- **Динамические и цветные источники света, emissive** — выключены по умолчанию либо приходят от нативных модов/паков.
- **Отладочные gizmo и crosshair (F3)** — P2.
- **Пользовательские post-цепочки и спектаторский эффект** — P1.
- **Водное зеркало, GPU-поза, нативные паки-shadow** — только модерн-путь.
- **Полная панорама заставки** — V06, P2.
- **CIT, portal-шейдеры, pack text** — отклоняются (README `horizon-legacy-render`). V03.

## 8. Тестовые пробелы (подтверждено `absent`-проверками)

- нет legacy-теста на коды 11/12 (трещины) и readback трещин;
- нет legacy-теста на `outline`, `debug_lines`, `debug_crosshair`, `clouds`, `end_of_frame`;
- нет проводки `TerrainHudRenderer` и `set_glint_textures` в `Game`;
- есть отказ для 2/7/9/33/34/38 (`entity.rs`), для title/chunk fade/menu blur (`frame.rs`) — только как отказ, не как поддержка;
- нет прямых legacy-тестов на коды 5, 16–31 и 8 (покрытие косвенное или только modern);
- нет legacy-теста на `skin: Some(..)` (отказ по `unsupported()` не проверяется).

## 9. Запросы интегратору (по регламенту §4, как предложения, не правки)

1. **F01:** профиль legacy — явные значения `chunk_fade`, clouds, menu blur, title; лог решений. Contract version не меняется.
2. **Семантика `end_of_frame`:** проверка наличия цепочки в активном root до запроса.
3. **Проводка glint:** вызов `set_glint_textures` из загрузчика ресурсов в адаптере Game (A02/E01).
4. **Пасса для линий/контура (V01) и трещин (V02):** слоты dispatcher, порядок относительно translucent terrain и HUD; тесты readback.
5. **Энерго-вихрь (34):** решение «реализовать» или «выключить с объяснением».

## 10. Непроверено

- Реальные ассеты: диапазон UV моделей, существование `end_of_frame`, полный перечень screen-эффектов.
- Поведение Rust-кода: тесты из §4–§5 не запускались.
- Поведение трещин на GL2: policy перенесена из modern-пути без readback.
- Win7/X1650: не проверено.
