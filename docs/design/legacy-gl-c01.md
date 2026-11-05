# C01 v1: подписанные границы первого интеграционного слоя

2026-11-04. Владелец: главный интегратор Arena.ai Agent `lukintrees`.
Этот документ фиксирует **реально добавленные** API и оставшиеся шаги, а не объявляет весь
Game переведённым на legacy. `CONTRACT_VERSION = 1`; текущие M01a/O01a/T01a/D01a — независимая
подготовительная волна. Полный R01/R02/R03/G01 пока не завершён.

## 1. Точные пути и API

| Путь | Контракт v1 / статус |
|---|---|
| `crates/horizon-render-api/src/render_contract.rs` | `Backend`, `FrameOutcome`, generations/stamps, feature negotiation, `VisibleSections`, `FrameOptions`, CPU `ScreenshotRequest` |
| `crates/horizon-render-api/src/resource_contract.rs` | Borrowed `Resources`/`TextureLayers`, exact-length/overflow/aggregate base-pixel budget validation; ownership/peak staging остаются у A01 |
| `crates/horizon-render-api/src/platform_contract.rs` | `WakePolicy`, normalized event vocabulary/USB HID usages, pointer mode и `PlatformWindow` producer seam |
| `crates/horizon-client-render/src/renderer_facade.rs` | Borrowed modern/legacy adapter; common render/size/resize/upload/capabilities через CPU `frame::Frame`; **без Deref и GPU-полей** |
| `crates/horizon-client-render/src/frame/` | R01a CPU `Frame`/`BorderDraw` и DTO для lightning, GPU-pose jobs и shader-pack inputs; в этом дереве нет `wgpu` handle types |
| `crates/horizon-client-render/src/gfx.rs` | Compatibility re-export `Frame`/`BorderDraw` и `ScreenshotRequest`; modern internals пока здесь |
| `crates/horizon-client-render/src/gfx/{entity_skin,lightning,pack_draw,pack_uniforms}.rs` | Прежние public пути re-export CPU DTO из `frame/`; GPU pipelines/buffers остаются modern-owned |
| `crates/horizon-client-render/src/legacy_gfx/frame.rs` | `Options`/`VisibleSections` compatibility re-exports из CPU contract; прежний preflight сохранён |
| `crates/horizon-client/src/client_lib.rs` | Общие модули доступны Game через прежний client re-export chain, без новой Cargo dependency |
| `crates/horizon-client/src/game/event_loop.rs` | Game-facing `EventLoopCtl` принимает **нейтральный** `WakePolicy`; преобразование в winit только в adapter impl |

Только интегратор меняет эти общие точки. Полный executable protected-path список:
[`.github/legacy-gl-policy.json`](../../.github/legacy-gl-policy.json).
`render_contract` и `platform_contract` не создаются повторно в другом crate.
Граница «общий CPU-слой не называет wgpu handle types» проверяется
`tests/test_legacy_cpu_contracts.py` (комментарии разрешены, код — нет); полный
wgpu-free dependency graph остаётся за D01.

### Владение и ошибки

- Modern owner пока прежний `gfx::Renderer` в Game/Loading/Missing. Facade берёт эксклюзивный borrow
  на одну операцию. Это безопасная ступень миграции, **не** завершённый owning facade R02/R03.
- Legacy owner — существующий native context/window + `TerrainHudRenderer`, GL calls на том же потоке.
  Не создавать дополнительное winit-окно ради legacy; не подделывать native/winit IDs.
- Resources заимствуют CPU pixels без второй постоянной копии. Успешный полный reload публикует новую
  generation; failed staging её не меняет. Base-byte validation не заменяет peak CPU/VRAM budget A01.
- `SectionStamp` сравнивает world/resource generation и строго растущую revision; W01 хранит tombstones
  и ограничивает очереди. Facade raw upload сам пока **не** реализует job queue/stamp enforcement.
- `Presented` ≠ `Offscreen` ≠ `Deferred`. Modern уже presents внутри render; legacy presents ровно один раз
  после успешной подготовки/draw. Unsupported frame возвращает ошибку, не «успешный пустой кадр».
- Screenshot — команды фасада (`request_screenshot`, `screenshot_pending`, `take_screenshot_request`,
  `take_screenshot_result`, `restore_screenshot_result`): Game не пишет поля renderer'а напрямую, а
  device-recovery/pack-reload переносят pending request/result через них. Modern снимает кадр сам;
  legacy с U01b выполняет bounded native-resolution SDR PNG / centre-cropped icon после draw,
  до present. Supersampling отклоняется явно; request/result можно забрать при замене owner (§8).
- Surface handover (`release_surface`/`recreate_surface`) — тоже команды фасада: modern передаёт и
  восстанавливает wgpu surface, legacy возвращает ошибку, потому что его surface принадлежит native
  context (G01). Молчаливый no-op оставил бы два владельца окна.
- Diagnostics (`gpu_info`, `lod_distance`, `drawable_size`, `present_mode`) — нейтральные функции
  фасада: Game и `horizon-ctl` больше не называют `wgpu::AdapterInfo`/`SurfaceConfiguration`, а F3
  получает вид GPU как `DeviceClass`, а не `wgpu::DeviceType`. Legacy заполнит ту же форму из GL-строк,
  когда G01 начнёт классифицировать адаптер; до тех пор дальность не урезается.
- Terrain upload (`upload_section`) — команда фасада: Game больше не вызывает `Renderer::upload`
  напрямую; ошибка (legacy) логируется, а не теряется молча.
- **R02g (2026-11-05):** clear/remove/query terrain и Game/loading/reload resize проходят через
  facade. Legacy adapter имеет настоящие clear/unload/contains/count operations, а не no-op stubs.
  Game section Y — индекс относительно dimension minimum: перед первым legacy Game upload обязателен
  `clear_sections(ColumnLayout)`. Без layout или при Y за его пределами upload отклоняется до GPU mutation.
  Low-level zero-origin probes по-прежнему используют `upload_section` напрямую.
- Native scene добавляет `min_y` к Y origin **до** camera subtraction/f32 conversion. Reset проверяет
  alignment и pending cleanup до смены origin/membership; textures/lightmap/atlas сохраняются.
  Column unload удаляет все Y только заданного X/Z, ошибки cleanup сохраняются для retry. Batch removal
  не транзакционен: после частичной ошибки будущий owning legacy adapter обязан остановить draw/recover,
  а не подтвердить успешную смену мира. W01 generation/revision queue здесь ещё не реализована.
- `capabilities()` описывает реализованные feature families, **не** фактические hardware GL limits.
  `require()` явно отклоняет GPU poses/panorama/packs на legacy; F01 ещё должен вызвать negotiation
  до geometry generation. Native driver checks/budgets продолжают проверяться существующим backend.
- Minimize приходит от native owner как нулевая drawable size: facade пропускает draw. Explicit native
  zero backing resize возвращает ошибку; это не скрытое allocation и не остановка simulation/save.
- Text events — committed Unicode; physical keys — HID page 0x07 usages, не Win32 scancodes/winit IDs.
  P01/P02 ещё реализуют mapping, IME/platform details и bounded event queue adaptation.

## 2. Что уже подключено к настоящему Game

Все восемь прямых renderer submission call sites (world/loading/save/reload/missing resources) используют
`renderer_facade::{render_modern, render_menu_modern}`. Ошибки и прежняя offscreen bool semantics для существующего
`frame_drawn` сохранены. Offscreen outcome не переименован в окно, показавшее кадр.
Headless и winit adapters используют один `WakePolicy`; Poll/Wait/WaitUntil semantics сохранены.

**Обычный backend selection остаётся modern.** Нет нового рабочего CLI legacy startup и нет
утверждения «wgpu device больше не создаётся». Native facade проверяется отдельным настоящим GPU fixture,
а не имитацией запуска всей игры.

## 3. Карта оставшихся прямых обращений

Полный воспроизводимый [syntactic inventory](legacy-renderer-inventory.json) содержит исходные строки,
пути/line numbers и SHA256 файлов. Генератор: `python3 tools/ci/legacy_renderer_inventory.py`,
проверка актуальности: тот же вызов с `--check` (в CI01 policy-tests).
Он намеренно помечает `r`/`old`/`previous` accesses как **кандидаты**: это не Rust type/name resolver.
При изменении строки интегратор обязан проверить binding, не считать совпадение доказательством GPU use.

| Семейство / основные consumers | Следующая граница / владелец |
|---|---|
| `Game.renderer`, `Loading.renderer`, `no_resources::Missing.renderer` | R02/R03: owning facade, shared state migration — интегратор |
| `build_renderer`, `loading_renderer`, `renderer_for`, `new_renderer`, `recover_device` | R02/R03/G01: factory, ownership, reload/recovery — интегратор |
| `config`, `suspended`, window sizing / resize handlers | R02b/c: размер drawable через `renderer_facade::drawable_size`, surface handover через команды фасада; Game/reload resize уже команда facade (R02g); полноценный native minimize loop остаётся P01 |
| `device`, `queue`, `hud` uploads в skins/maps/UI/glint | A01/A02/E01/R02: CPU patches/resource commands; не протаскивать device/queue в общий trait |
| `screenshot*`, capture requests/results, device recovery | R02a: facade commands `request_screenshot`/`screenshot_pending`/`take_*`/`restore_*` уже используются Game; modern capture остаётся в modern adapter; native SDR readback/completion — U01b (§8) |
| `info`, gpu times/census, HDR calibration, shader macros/packs | Backend-neutral diagnostics; modern-only commands явно capability gated |
| `upload`, section membership/removals, `take_dropped_sections`, LOD / culling / static entities | W01/R02: upload/clear/remove/has/count уже методы facade; native Game key Y учитывает dimension minimum (R02g). W01 stamps, bounded queues, CPU visibility и modern GPU paths остаются |
| `skins_entities`, `skinned_models`, GPU posing storage | E01: настоящее CPU posing, не только `skin=None` |
| `gamma`, fullbright, animation/filtering, fog/lightmap | F01/A02: policy/settings commands, не публичные GPU-owned поля |
| `frame::Frame` и DTO `skin`/`lightning`/`pack` | R01a вынесены в общий CPU-модуль; старые `gfx::*` пути совместимы. Остаток R01: visibility/resource descriptors и отделение dependency graph, а не второй Frame |
| winit `Arc<Window>`, WindowEvent/KeyCode handlers | P01/P02/P03/P04/G01: platform adapters; `WakePolicy` уже вынесен, остальной loop ещё нет |

## 4. Что не замораживаем ложным обещанием

В этой ступени **ещё не реализованы** оставшиеся visibility/resource descriptors и wgpu-free
dependency graph, resource reload completion API, owning factory/renderer selection, native supersampled
screenshots, полноценный platform adapter и Game legacy startup. Menu/static path и bounded native SDR
screenshot completion добавлены в U01a/b (§7–8). Их точные новые signatures согласует и внесёт интегратор;
нельзя самостоятельно добавлять конкурирующие common types в task PR.

## 5. `status.legacy` — хук подтверждения запускаемого legacy (G01/T01a)

`Game::status` (control) всегда публикует объект `legacy`: `confirmed`, `backend`,
`gl_version`, `glsl_version`, `vendor`, `renderer`. Пока окно владеет modern wgpu renderer,
значение честно `confirmed=false` с пустыми GL-полями; runner T01a трактует это как
unconfirmed/blocked, а не как успех. Когда G01/R03 подключит настоящий legacy путь, те же поля
заполняются из реального контекста (`glGetString`), и `status.legacy.confirmed` меняет значение.
`wgpu`-бэкенд `renderer.backend` (включая `Gl`) никогда не является legacy-подтверждением.

Это начальная C01 v1 реализация и рабочий submission wiring, не закрытие всех критериев C01/R01–G01.
Для четырёх подготовительных задач достаточно этой границы и scopes из промптов. Перед основной
adapter-волной требуется следующий интеграционный шаг C01/R01 с объявлениями готовых operations.

## 6. R02g: проверка и открытые gates (2026-11-05)

Продолжение после `b00954c` (R02f), без повторной реализации принятых R01a/O01a/T01a/R02a–f.
Расширенный independent `backend_resident_sections` теперь проверяет отрицательные/положительные
минимумы dimension, invalid reset, column unload и повторные reset/upload в ограниченном mesh budget.

Локально: **43 Rust tests**, workspace all-targets Clippy на host и i686 Windows target,
**3×26 OSMesa cases** (gl20/gl21/native), **679 Python tests, 26 skipped**;
rustfmt, inventory и public-tree checks. Новый facade GPU fixture также добавлен, но локально не запускался:
full client требует другой toolchain/dependency environment.

Пользователь включил Actions; прежняя блокировка HTTP 422 снята. Собственные focused runs
[41964269960](https://github.com/lukintrees/horizon/actions/runs/41964269960) (`904e30d`, dispatch),
[41964590531](https://github.com/lukintrees/horizon/actions/runs/41964590531) и
[41964596387](https://github.com/lukintrees/horizon/actions/runs/41964596387) (`0dbd7bd`, push/PR)
прошли успешно: modern + legacy full-client compile, 17 game GPU fixtures, hosted x86 и aggregate.
Это подтверждённый R02g full-client baseline, но не Win7/X1650 runtime qualification.
Required-check protection не устанавливалась; draft остаётся draft.

По запросу пользователя в `0dbd7bd` 21 неиспользуемый workflow (parity, launcher, labels, release и т.д.)
перенесён в `.github/workflow-archive/`, зарегистрированные ненужные workflows отключены.
В активной директории оставлены focused branch validation и trusted scope gate; modern проверки
сохранены внутри focused validation. Тест allowlist не допускает случайного возврата лишнего CI.
Trusted-target bootstrap/default-branch protection по-прежнему отдельный незакрытый gate.

## 7. U01a: явный menu request, не legacy startup (2026-11-05)

- `frame::MenuFrame` содержит только HUD, background и panorama view: никакой world frame не
  обрезается адаптером. `MenuBackground::Static([f32; 4])` — явный gamma-encoded SDR clear
  в non-sRGB framebuffer; диапазон/finite values и размеры HUD проверяются до clear.
- Facade `render_menu` поддерживает modern panorama/static и native static. Legacy panorama
  возвращает capability error, не подменяется фоном молча. Native empty visibility не удаляет
  resident terrain/atlas/lightmap. GUI conversion/validation выполняется до framebuffer mutation.
- Семь loading/save/no-resources submissions используют menu helper; canonical `Frame::menu` перенесён
  из loading constructor без изменения default panorama. Основной world/title submission пока остаётся
  полным Frame. Game всё ещё владеет modern renderer; F01/G01 обязаны выбрать static profile явно.
- Modern static background прокинут per-call через обычный и scaled-screenshot draw; persistent
  override отсутствует. Scene-capture format пока не представляет static background: запрос получает
  явную ошибку, а не неверный panorama replay. Native screenshot command completion на этапе U01a оставался открытым; U01b описан ниже.
- CPU tests покрывают canonical empty menu, invalid view/color/layout и stale HUD после resize.
  Новый real-GL facade fixture проверяет clear/HUD pixels, отказы до clear, resize и возвращение к
  resident world после menu. Теперь в focused workflow **18** ignored game GPU fixtures;
  статус конкретного нового head следует проверять по его собственным Actions runs, не по R02g.

Локальная проверка native workspace: **44 Rust tests**, host Clippy и legacy Python checks.
Full-client compile/GPU и hosted i686 выполняются focused Actions; локальный Rust target i686
в текущем окружении недоступен. Ни B1–B4, ни owning Game/native factory, ни hardware qualification
этот этап не закрывает.


## 8. U01b: bounded native screenshot completion (2026-11-05)

- CPU `horizon_render_api::screenshot::Plan` проверяет nonzero/overflow/size и combined source+icon
  byte budget **до GL readback allocation**. Full-frame RGBA буфер переворачивается in place из GL
  bottom-up в top-down и становится opaque; centre-square icon усредняется с широкими RGB sums.
  Это CPU-only код без GPU handles, codec и filesystem; общий guard сканирует и этот файл.
- Native owner хранит один pending request и один completion; borrowed facade recreation их не теряет.
  Busy/невалидный запрос не перезаписывает существующий. `take_*` одноразовые; explicit restore result
  и перенос взятого request работают при owner replacement. Modern request/result semantics не менялись.
- После успешного world/menu draw facade завершает request **до swap/present**. Rejected/deferred draw
  оставляет его pending. Readback/PNG/flush error публикуется как screenshot result, не ломает успешный
  frame outcome. После resize budget проверяется повторно; постоянного readback буфера нет.
- `Limits::screenshot_bytes` по умолчанию **16 MiB** на source + icon RGBA; PNG codec/stream overhead
  сюда не входит. Supersampling (`scaled=true`, без explicit icon size) и upscaling отклоняются явно.
  F01 обязан выбирать native-resolution policy для legacy F2; normal modern F2 не менялся. Это не
  поддержка HDR/scene replay и не подключение Game к legacy owner.
- CPU workspace tests проверяют orientation, opacity, centre crop/average, exact byte limits, overflow,
  bad lengths и reuse full-frame allocation. Native queue test проверяет bounded slots/rejection.
  Новый OSMesa facade fixture декодирует реальные PNG menu/world/icon, проверяет orientation/alpha,
  pending/result/handover, rejected frame, I/O failure и resize-budget failure/recovery. Теперь focused CI
  запускает **19** ignored game GPU fixtures; новый baseline требует exact-head green checks в PR.

Локально: **47 native workspace Rust tests**, host all-targets Clippy, legacy Python guard tests,
format/inventory/public-tree checks. Hosted x86 и full-client/19 GPU fixtures проверяются Actions.
B1–B4, F01/G01/resource completion и настоящая Win7/X1650 qualification остаются открыты.


## 9. A01b/R03a: native factory из CPU resources (2026-11-05)

- `resource_contract::TerrainAtlas` принимает borrowed layer-major RGBA8 из общего `Resources`.
  Layer ID остаётся индексом placement; high-bit mesher material flags не меняют индекс текстуры.
  Padding=1 extrudes края. Проверяются exact pixels, размеры, max layers (не более 32768), pages,
  page+slice-table staging bytes; defaults: 512² page, 128 pages, 2 MiB staging. Layout metadata
  отдельно bounded layer count; это не полный peak memory budget A01/Q02.
- `WorldScene::new_paged` подготавливает и загружает ровно одну страницу за раз. Borrowed slice
  constructor сохранён. Preparation/length/upload failure уничтожает partial owner и возвращает
  ошибку; ранее созданные textures удаляются. Тесты fake backend проверяют lazy ordering и cleanup.
- `TerrainHudRenderer::from_resources(Context, Resources, lightmap, Limits, ResourceLimits)` создаёт
  native owner без wgpu initialization/modern PreparedTextures. Bundle validate (default 128 MiB base)
  и минимальная GPU residency (terrain+lightmap+white+2 glints+1 HUD page) проверяются до backend upload.
  Actual driver/budget checks остаются backend-owned. HUD backing compressed; полная копия terrain
  в owner не сохраняется. Старый low-level constructor совместим.
- Успешный factory загружает оба glint images и публикует `ResourceGeneration`. Facade query возвращает
  Some только для owner с такой provenance; старые modern/synthetic constructors возвращают None,
  не выдуманную generation=0. Failed factory не публикует replacement и не меняет active owner.
- CPU+OSMesa fixtures проверяют стабильные IDs на нескольких страницах (red/green terrain pixels),
  HUD из bundle, сохранность resident sections, generation и failed-construction isolation.
  Focused CI теперь включает **20** ignored game GPU fixtures и CPU factory preflight test.

Это **initial factory**, не transactional reload и не Game startup: Game всё ещё modern-owned.
Pack decode/borrow extraction, animation updates/font-cache reset при commit, F01 policy negotiation,
resource replacement/context recovery и native platform loop остаются открыты. Реальный pack/hardware
не заменяется синтетическими fixtures: B1–B4/Win7-X1650 qualification не заявляются.


## 10. A01c/R03b: snapshot producers и glyph-cache commit (2026-11-05)

- `legacy_gfx::AssetSnapshot` берёт **существующие** `RenderAssets`/`HudAtlas`, не вводит второй pack
  loader. Terrain и оба glint buffers borrowed; layer IDs и source provenance сохраняются.
  Полную `ResourceGeneration` явно передаёт owner: HUD-only reload может изменить layout без смены
  `RenderAssets::generation`; snapshot отдельно сообщает terrain generation и HUD revision.
  Combined base-byte budget проверяется до ожидания/распаковки HUD. Snapshot — явно base-frame SDR;
  animation counts доступны отдельно, live animation loop этим не реализован. Custom core text/line
  shaders отклоняются, не теряются молча.
- `HudAtlas::checked_pixels` — strict native staging path: dimensions/exact length/base budget,
  LZ4 header и bounded decompression в заранее ограниченный buffer. Corrupt input возвращает ошибку,
  не transparent fallback. Старый `pixels()` и default modern behavior не менялись.
- `TerrainHudRenderer::stage_assets` использует snapshot и initial CPU factory, но не сбрасывает
  активный font cache. Failed/discarded stage освобождает свой owner, не меняет epoch/UV/patches.
  `StagedAssetRenderer::commit` — явная publication boundary: changed font epoch отклоняет stage;
  успешный commit сбрасывает cached glyphs **и старые pending patches**, после чего нужно заново
  построить text/HUD batches. После reset нет fallible GPU upload в commit.
- CPU fixtures проверяют borrow provenance, generation/base budget, raw/packed HUD roundtrip,
  malformed headers/payloads и glyph re-rasterization после renderer commit. Новый OSMesa fixture
  использует RenderAssets fixture + packed HudAtlas, проверяет fail/drop/stale-epoch/commit и настоящий
  Unicode glyph patch в framebuffer. Теперь focused CI исполняет **21** ignored game GPU fixture.

Это producer/factory seam, а не вызов из Game startup. Native owner publication в Game, platform loop,
animation updates, transactional reload всех ресурсов, F01 negotiation и Win7/X1650 qualification ещё
открыты. Синтетический font fixture — не импорт Mojang assets и не доказательство запуска полного pack.


## 11. A02a: explicit native terrain/HUD animation maintenance (2026-11-05)

- `stage_assets` устанавливает контроллер из **существующих** `SpriteAnimation`. Clone разделяет Arc
  с исходными frames/clip; второго parser/pack loader/таймера нет. Команда owner/facade
  `advance_animations(full_bundle_generation, tick)` использует существующий `blend(tick)`, nearest
  resampling и целочисленное `(a*(1000-millis)+b*millis+500)/1000` для всех RGBA channels. Первый tick
  явно загружается; повторное sampled state не выделяет pixel buffers и не пишет textures.
  Modern отклоняет эту maintenance-команду: его прежний in-draw animator не изменён.
- Bounded `atlas::sprite_patch` extrudes gutters. `WorldScene::update_sprite` меняет только texture
  subrectangle существующей страницы: layer IDs, placements/UVs, sections и VBOs не пересоздаются.
  HUD использует существующий transactional compressed `Backing::patch`, затем инвалидирует только
  изменённые cached pages; padding/gutters обслуживает прежний paging path.
- Native defaults: **4096** animated targets, **65536** source frames, **32 MiB** conservatively
  charged shared RGBA frames, **8 MiB** update staging. Повторно используемые frames могут считаться
  несколько раз: это безопасная верхняя оценка, не новый physical copy. Проверяются target IDs,
  duplicate terrain layers, finite/in-bounds/nonoverlapping HUD rectangles, непустые одинаковые
  RGBA-square frames и checked arithmetic. Invalid installation не публикует owner/font-cache reset.
- До allocation/upload changed pixels суммируются для terrain + HUD; к ним добавляется максимум из
  одного padded terrain scratch и union decoded dirty HUD pages. При HUD patch retained terrain bytes
  вычитаются из доступного working limit. Metadata ограничена count/page limits; codec/compressed
  staging overhead, уже загруженные producer metadata и полный process/VRAM peak сюда не входят.
  Compressed backing и GPU residency по-прежнему имеют отдельные budgets. Это не закрытие Q02/D01.
- Foreign/untracked generation и healthy preflight failure ничего не меняют. После возможной частичной
  upload/patch failure world/menu/Game-frame draw запрещён до успешного retry, без clear предыдущего
  framebuffer. Retry принудительно обновляет **все** animations, даже если desired state совпадает
  с последним успешно отмеченным: иначе уже перезаписанная terrain texture могла бы остаться чужой.
- CPU fixtures проверяют interpolation/alpha/nearest, custom frame sequence, wrap/large tick,
  repeated-state skip, Arc sharing, combined exact staging/page budget и malformed inputs. Новый OSMesa
  fixture использует реальные RenderAssets/HudAtlas и SpriteAnimation producer, проверяет animated
  terrain рядом со static neighbour, HUD/cache invalidation, tick 0/half/step/wrap/repeat, stale bundle,
  stage/preflight failures и **настоящий HUD compressed-budget отказ после terrain upload** с полным
  восстановлением при retry старого tick. Focused CI теперь запускает **22** ignored game GPU fixtures.

Это native maintenance seam, **не live Game animation loop**: Game по-прежнему modern-owned, snapshot
без явного clock command остаётся base frame. Owning native startup/authoritative clock wiring,
transactional reload/context recovery, F01/G01/W01 и Win7 x86/X1650 qualification остаются открыты.
Ни A02 целиком, ни B1–B4, ни playable pack/release этот шаг не объявляет завершёнными.


## 12. A02b: transactional same-context native resource replacement (2026-11-05)

- `TerrainHudRenderer::stage_reload_assets` получает existing producers, expected/next full-bundle
  generation, authoritative tick и preparation limits. Native `StagedAssetReload` эксклюзивно заимствует
  active owner: во время synchronous staging нельзя рисовать, resize или аллоцировать его ресурсы.
  При drop/error прежние generation, terrain/HUD/glint/animation data, sections, screenshot slots и
  font cache не публикуются заново. Base framebuffer не очищается. Source font change отклоняет commit.
- Backend candidate использует **тот же Rc-owned Context/window**, но отдельные shader/resource IDs.
  Staging payload/object caps равны оставшейся capacity active backend, не второму полному GPU budget.
  Old + candidate RGBA/VBO payloads и tracked objects поэтому bounded во время подготовки. Это
  дисциплина exclusive owner guard, **не глобальный ledger для произвольных сторонних GL owners**;
  shader/program/driver overhead и реальный VRAM peak не приравниваются к payload accounting.
- Source targets/frame counts/shared RGBA и compressed HUD backing также получают только остаток
  соответствующего budget после active usage. Arc-shared source может conservatively учитываться
  дважды. Нет destructive fallback «удалить старое и попробовать»: insufficient joint capacity — ошибка.
  Preparation base pixels/update scratch/codec overhead по-прежнему имеют отдельные границы; полный
  process peak и ресурсы внешнего Game/pack loader этот шаг не ограничивает.
- До commit candidate уже содержит новые terrain, HUD, оба glints, copied lightmap и animations sampled
  на requested tick; dimension min_y сохранён. Все fallible validation/budget/cache promotion завершены
  **до** glyph reset/publication. Commit переносит оба screenshot slots без повторного request/I/O,
  сбрасывает cached glyph UVs/pending patches нового producer и заменяет owner. Старые meshes/cache/
  resources уничтожаются; никакой UV-bearing section не переживает смену layout/generation.
  Нормальные full limits и HUD cache capacity восстанавливаются, staging cap не накапливается.
- Публикуется только строго более новая full-bundle generation; anonymous/foreign/reused/older commands
  отклоняются. Caller обязан опубликовать matching CPU producers, перестроить text batches/world meshes
  и stamp/filter streaming jobs. Этот native command сам не реализует W01 stale job scheduling.
  При `legacy-gl` включён и synchronous `RendererFacade::reload_assets`; modern adapter его явно
  отклоняет и сохраняет прежний Game-owned reload path. Default modern startup не меняется.
- CPU checks покрывают generation/capacity subtraction и checked backing-limit promotion. Новый
  OSMesa fixture проверяет context identity, rollback/drop/stale font, joint GPU/source exhaustion,
  partial staged resource cleanup, unchanged old pixels/sections, commit/animation tick/layout min_y,
  transferred screenshot request+completion и повторные reload cycles со стабильной tracked residency.
  Существующий Unicode fixture расширен реальной glyph re-rasterization после same-context reload.
  Focused CI теперь выбирает **23** ignored Game GL2 GPU fixtures.

Это synchronous native renderer transaction, **не full Game pack reload**: CPU pack/language publication,
streaming queues, native Game startup/platform input/clock wiring ещё требуют A02/F01/G01/W01.
Context loss/recreation — отдельный lifecycle, не «recover» через текущий possibly-lost context.
Window identity проверена shared-context ownership/readback, но interactive Win32/Win7/X1650 и Alt-Tab
этим OSMesa fixture не квалифицируются. B1–B4 и release не объявляются завершёнными.


## 13. W01a: bounded owner-issued terrain tickets (2026-11-05)

- CPU-only `horizon_render_api::streaming::Stream<T>` выдаёт существующий `SectionStamp` **до dispatch**
  mesher job. Completion принимается только с точным текущим ticket; foreign world/resource, unissued,
  superseded и duplicate work возвращают `Stale`. Revision — owner-issued nonce, строго возрастающий
  в пределах world/resource epoch, не stamp, присвоенный задним числом готовому mesh.
- Интерес к section, очередь и payload bytes ограничены независимо. Native defaults: **4096 active
  tickets**, **64 pending completions**, **8 MiB retained mesh input**. Queue count/byte pressure и
  exhaustion interest slots видны caller; rejected Arc остаётся у producer. Complete на full queue
  можно повторить после drain без нового ticket. Новая request отменяет старый queued completion,
  но не прежний resident GPU mesh. FIFO не сортирует геометрию/visibility вместо Game.
- Tombstone не требует бесконечной таблицы всех посещённых координат: unload сначала отзывает lease,
  а metadata освобождается только после acknowledgement GPU membership removal. Readmission того же
  key выдаёт новый глобальный nonce, поэтому поздний прежний job не может воскресить section даже
  после удаления записи. Pending removal запрещает draw до retry/drain/reload; stale unload ticket
  не удаляет более новый lease. Empty completion освобождает GPU membership, сохраняя interest lease.
- Native owner/facade имеют opt-in begin/request/queue/drain/unload/usage commands. Begin требует
  tracked resources и более новый world/teleport epoch при повторном вызове. С этого момента старые
  unstamped upload/clear/remove APIs явно отклоняются — обойти ledger через старый facade нельзя.
  До opt-in прежние probes/adapters совместимы. Modern scheduler/path не переключаются.
- Native очередь разделяет `Arc<SectionMesh>` настоящего producer, не копирует vertices и не вводит
  второй mesher. Учитываются **capacity**, а не только len, всех seven Vec buffers: solid/translucent,
  packed/tints, block entities/lights и sprite layers, плюс payload/header. Shared buffers могут
  conservatively считаться несколько раз; allocator/external worker overhead не является process
  budget. Far-mesh keep/fade отклоняется. Packed-only data не заменяет full terrain vertices.
- Drain имеет hard job-count и charged-input-byte limits, без «хотя бы один oversized upload».
  Нулевой count ничего не делает; нулевые bytes допускают только removals. Oversized FIFO head ждёт
  большего explicit budget/new request. Это не elapsed-time/driver/converted-scratch budget: conversion
  отдельно ограничена section_vertices, garbage cleanup/object counts — native scene/backend limits.
  Upload failure сохраняет старый GPU mesh и queued completion для retry/replace/revoke.
- Failed/discarded resource reload сохраняет очередь. Commit A02b публикует пустой armed stream с тем
  же world, новой resource generation и без старых jobs/leases. World reset отменяет pending work;
  заново использовать world/resource пару после owner recreation нельзя без переноса ticket namespace
  или смены generation. Полный context recovery остаётся отдельной задачей.
- Standalone CPU tests покрывают stamps, capacity/byte backpressure, FIFO/coalescing, ack/retry,
  revocation/readmission (1000 cycles), epoch rebase и nonce exhaustion без wrap. Native CPU test
  проверяет все retained capacities. Новый OSMesa fixture использует настоящий SectionMesh через
  facade: dimension origin, queue/interest pressure, hard drain limits, **реальный backend replacement
  budget отказ** с сохранением старых pixels, revoked draw guard, unload/revisit, failed/drop/commit
  reload, world change, 32 travel/unload cycles и last-block/air completion. Focused CI выбирает
  **24** ignored Game GL2 GPU fixtures.

Это native streaming control plane, **не wiring существующих Game workers**: modern pending_uploads,
frustum/cave visibility, camera sorting и simulation не меняются. W01b/G01 должны выдавать tickets
до dispatch настоящим workers, публиковать matching CPU world/resources и выбирать native owner.
Native startup/input/clock, context recovery, Win7/X1650 qualification и B1–B4 остаются открыты.


## 14. MVP 1a: owning storage в настоящем Game (2026-11-05)

Порядок дальнейшей работы — [последовательный MVP-план](legacy-gl-mvp-path.md).
`Game.renderer` теперь `renderer_owner::RendererOwner`: Empty / boxed Modern / feature-gated
boxed Native. Storage не создаёт device/context, не имеет Deref/GPU-полей в public API и
заимствует прежний `RendererFacade` с caller-supplied visibility/options. Modern leaf paths
явно используют `modern_ref/modern_mut/has_modern/take_modern`. Их `None` **не** означает,
что native frame успешно нарисован. Native-to-modern publication/initialization запрещена;
`take_modern` не извлекает и не теряет native owner. Native submission в `Game::render` пока
явно возвращает ошибку, modern winit resume/factory не принимает native owner.

Это первый ownership patch этапа 1, **не его закрытие**. Startup loading owner остаётся
modern-only; флаг выбора, native Game bootstrap/profile и runtime dispatch ещё не подключены.
Настоящий Game worker dispatch/visibility/input/save путь ещё modern. Новая offscreen owner
fixture проверяет ownership/facade, а не menu/playable runtime. Default feature graph не менялся.


## 15. MVP 1b: native bootstrap route, не playable Game (2026-11-05)

`Options.renderer` отличает `modern` от `native-gl2`; client feature `legacy-gl` только
включает native adapter, не отделяет dependency closure. Native требует явный
`--native-bootstrap-only`. Без feature/с несовместимыми аргументами — usage error до
`Options::read` startup side effects, без wgpu fallback. Existing Launch API не менялся.

Общий synchronous loader теперь получает startup profile: native UI строит настоящий GUI
atlas, не декодирует panorama, не включает LabPBR, не вызывает modern PreparedTextures даже
при будущем menu handoff. Native bootstrap сознательно vanilla-only и не запускает внешние
моды. Saved renderer/packs/HDR picks не меняются. Modern Loading отказывает native selection,
пока не подключён его native owner/lifecycle, а не пытается передать wgpu surface.

Настоящий `start_game` строит Game и устанавливает native owner из его `pack.render` и
`gui.atlas` через `RendererOwner::legacy_menu_bootstrap`. Initial Title helper общий с modern.
`native_startup::render_menu` вызывает **Game::hud**, затем existing facade, static SDR menu.
World/loading/reload state отвергается; Offscreen остаётся не Presented. Native bootstrap
не входит в simulation/save/normal event loop. Это промежуточная диагностическая ветка,
**не закрытый runtime gate 1/2**, не альтернативный engine. [Инструкции и evidence boundaries](legacy-gl-native-bootstrap.md).

Новый factory тест использует CPU RenderAssets/HudAtlas fixtures. Он не заменяет полный
Game resource-load/runtime прогон, который здесь пока невозможен без normal prepared folder.


## 16. MVP 2a: GLX window и native menu adapter (2026-11-05)

`Context::new("glx", size)` — desktop legacy GLX context + собственный mapped X11 window.
Dlopen X11/GL/xkbcommon, без новых Rust dependencies; EGL/OSMesa и modern default не меняются.
GL2 не подменяется GLES/wgpu. Lifecycle: bounded 256-native-message pump, existing bounded
queue/coalescing/overflow notification, focus/capture reset, minimize/restore, WM_DELETE
persistent close; context/window/colormap/display cleanup и thread confinement.

`WindowEvent::X11Key` явно несёт X11 keysym/keycode, **не Win32 scancode**; старый `Key`
остаётся Win32. Unicode decoding через xkbcommon, без process-global setlocale. XIM/compose
не реализованы. GLX test проверяет back-buffer GL readback, native swap и XGetImage front
pixels; input/repeat/focus/minimize, repeat context teardown/window counts. Это window fixture,
не actual Game runtime acceptance.

Фасад получает `poll_native_window_events`, modern отвечает явной ошибкой (его owner — winit).
Native Game menu pump использует existing Game::hud и Title navigation/edit/hit tests, без
fake DeviceId/WindowId и без Game simulation clone. Unsupported world/options/external
producers показывают Title::failed до world entry. Native startup без bootstrap flag теперь
выбирает видимый menu-only loop; offscreen ограничен bootstrap-only. Lifecycle/platform
часть этапа 2 начата, responsive loading/full UI/gameplay и actual-resource runtime evidence
ещё открыты. См. [команды и ограничения](legacy-gl-native-bootstrap.md).


## 17. MVP 2b: responsive native loading и same-context handoff

Первый срез MVP 2b (`904da91`): main-thread Context/borrowed LoadingScreen → actual boot HUD before
worker dispatch → one bounded CPU completion → joined worker → drop boot GPU resources →
move **same** Context into existing native asset factory → Game/Title menu loop.
No native GPU handles/Context reach worker; no second window/modern renderer/PreparedTextures,
no terrain placeholder, no leaked Resources lifetime. Bootstrap-only diagnostic remains sync.

Cancellation belongs to loader generation (shared Progress flag); CPU checkpoints + joined
ownership prevent late publication/worker accumulation. Normal cancel continues pumping until
join; error-path Drop may wait for an uninterruptible phase. Full CPU load currently precedes
Title; на этом срезе GPU handoff был synchronous and measured. Текущий MVP 2c — ниже.
Boot and main-menu fonts/UI-theme cache publish at renderer commit; focus survives handoff.
`tools/native_game_menu_smoke.py` consumes real Game reports in fresh scratch dirs and cannot
accept bootstrap/offscreen/cancelled as observed native menu. No actual-resource runtime or
world/input/save/reopen/Win7 gate is closed by its unit mocks or the GLX/OSMesa fixtures.


## 18. MVP 2c: same-context cancellable GPU handoff

`WorldScene::new_paged_checked` adds owner-thread callbacks before/after page uploads;
callback errors use the existing backend error channel and Drop rollback. `StagePoint`
exposes coarse native startup boundaries, not a second engine/worker or async GL interface.
Owned LoadingScreen remains drawable while `staging_replacement_reserving` builds the
candidate within remaining tracked bytes/object slots minus bounded boot mesh headroom.
The existing zero-reserve reload method still requires a frozen old owner.

Final publication is boot retirement → budget/cache restoration → font epoch check/commit →
move complete renderer into empty Game owner. Game never draws an incomplete candidate.
Cancellation preserves the Game font epoch, reclaims both GPU owners and keeps worker join
proven separately from any presentation count. Shader/CPU/driver operations remain coarse,
non-preemptive; normal-assets runtime, hard frame-time guarantees, playable world/save and
Win7/X1650 acceptance are not inferred from fixture/CI results. See
[checkpoint/report semantics and test boundaries](legacy-gl-native-bootstrap.md#mvp-2c-event-aware-gpu-staging-и-atomic-final-publication).
## 19. MVP 3b: общий Game frame/idle/save routing

`Game::frame_tick` теперь содержит прежнее тело winit redraw. Native/headless
loop вызывают его через существующий `Game::idle`, без synthetic redraw IDs и
второго clock/frame-limit implementation. Native pump сохраняет bounded cadence
(16/32 ms; save deadline минимум 4 ms даже при pending messages) и сбрасывает
held UI input при focus/minimize/lost-event. Native pads пока не опрашиваются:
их menu input ещё зависит от winit events.

Loading/file-fixer/reload/save HUDs отправляются через owning facade с explicit
menu policy. Modern panorama и `Offscreen` completion сохраняются; native
minimize возвращает `Deferred`, не ошибку zero-sized HUD. Read-only chain
`WorldScene::backend` → `TerrainHudRenderer::size` → `RendererOwner::size` не
кэширует размер и не создаёт новое окно/backend; это compatible v1 addition.

Native Close/Quit/stop используют существующую durable save machine. Ошибка
остается ошибкой, save ownership не выбрасывается, visible pump termination
вызывает `Game::shut_down` и при ошибке. Native world/options/reconnect/deferred
entry/reload policy централизована в Game до неподключённых side effects.

`tests/test_game_loop_routing.py` компилирует реальные idle/title/resume/menu
методы с изолированными platform/storage/frame-boundary dependencies; focused
CI запускает его отдельно от actual Game library tests. Он проверяет routing,
refusal и error/completion outcomes, **не** реальные world writes/rendering.
Normal-resource Game не запускался; world submission/visibility/streaming/input,
runtime gates 1/2/3 и Win7/X1650 qualification остаются открытыми.

Локальная проверка этого increment (изолированные Python-created homes, исходники
стабильны): default/native `cargo check` и оба client Clippy `-D warnings` прошли;
2 event-loop + 2 headless + 7 startup tests в каждом профиле, filtered
`game::native_` — 124 passed/2 ignored, standalone render-api — 46 passed.
`pr_ready.py --quick --base 91488af43` прошёл public tree/fmt/Clippy/doc checks,
но **не green**: 2382 Rust tests passed, неизменённый
`lan::io_pool::tests::a_close_ends_a_disconnect_the_client_does_not_take` failed
(`still writing`, также 3/3 isolated reruns); Python — 689 run, один
`WinError 1314` при создании symlink в неизменённом vendor-security test.
Защитные тесты не ослаблены. Исходный WSL checkout дал 15 failures/1 error из-за
CRLF shell scripts/hooks и shebang `python3\r`; это не Game runtime evidence.
Новый commit требует собственного aggregate CI, а не результатов `91488af43`.

## 20. MVP 3c: native resident visibility/order seam

Compatible v1 additions: `WorldScene::section_keys` заимствует committed
membership без второго index, `min_y` возвращает committed Y origin.
`TerrainHudRenderer::section_keys` сначала проверяет pending-removal barrier;
revoked lease с незавершённым GPU cleanup не становится draw list.

`RendererOwner::native_visible_sections` заполняет caller-owned `SectionOrder`:
shared `streaming::in_view_distance` и `gfx::Frustum`, full 16³ AABB,
camera subtraction в f64 до f32, centre-distance/key ties nearest-first для
solid и обратный порядок для translucent. Scratch capacity ограничена actual
resident cap и резервируется fallibly; failed walk не публикует старые списки.
Borrow membership освобождается до facade submission. Cave graph и сортировка
translucent quads внутри секции не дублируются: последняя остаётся у producer.

CPU regressions покрывают permutation/order, camera walk, AABB/distance slack,
negative flooring, dimension origin, world-edge precision и failed-list clearing.
OSMesa regression scope — real `Frame`/facade readback, resident-only selection,
translucent order, camera movement, unload и смена world/Y origin. Это synthetic
geometry fixture, **не** запуск normal-resource Game. Game caller/world Frame,
live CPU poses, stamped Game streaming и gameplay input ещё не подключены;
world-entry guard, runtime gates 1/2/3 и Win7/X1650 qualification остаются открытыми.

CI01 scope policy защищает и `renderer_owner/**`, а не только owning `.rs`:
task edit нового visibility module отвергается; integration role разрешён.
Локально на стабильных Rust sources прошли 15 focused commands в default/native
профилях и standalone render-api (47 tests + all-targets Clippy). Повторный
`pr_ready --quick --base 1a12a2d51` с test `line-tables-only` прошёл public tree,
fmt/Clippy/doc checks и 1308 Rust tests, но **не green**: Python — 689 run,
один неизменённый `WinError 1314` symlink privilege error, 15 skipped.
Две unchanged crash source-location assertions падали только в validation с
debug=0: frozen-source A/B дважды воспроизвёл failure и дважды прошёл с render-package
line tables, без правок tests/production code. Новый OSMesa fixture локально только
compiled/ignored; actual execution требует собственного exact-head Linux CI.
