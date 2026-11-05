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
