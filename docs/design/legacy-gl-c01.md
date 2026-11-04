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
  legacy честно отвечает ошибкой «readback не реализован» до U01, а не молча теряет запрос.
- Surface handover (`release_surface`/`recreate_surface`) — тоже команды фасада: modern передаёт и
  восстанавливает wgpu surface, legacy возвращает ошибку, потому что его surface принадлежит native
  context (G01). Молчаливый no-op оставил бы два владельца окна.
- Diagnostics (`gpu_info`, `lod_distance`, `drawable_size`, `present_mode`) — нейтральные функции
  фасада: Game и `horizon-ctl` больше не называют `wgpu::AdapterInfo`/`SurfaceConfiguration`, а F3
  получает вид GPU как `DeviceClass`, а не `wgpu::DeviceType`. Legacy заполнит ту же форму из GL-строк,
  когда G01 начнёт классифицировать адаптер; до тех пор дальность не урезается.
- `capabilities()` описывает реализованные feature families, **не** фактические hardware GL limits.
  `require()` явно отклоняет GPU poses/panorama/packs на legacy; F01 ещё должен вызвать negotiation
  до geometry generation. Native driver checks/budgets продолжают проверяться существующим backend.
- Minimize приходит от native owner как нулевая drawable size: facade пропускает draw. Explicit native
  zero backing resize возвращает ошибку; это не скрытое allocation и не остановка simulation/save.
- Text events — committed Unicode; physical keys — HID page 0x07 usages, не Win32 scancodes/winit IDs.
  P01/P02 ещё реализуют mapping, IME/platform details и bounded event queue adaptation.

## 2. Что уже подключено к настоящему Game

Все семь прямых renderer submission call sites (world/loading/reload/missing resources) используют
`renderer_facade::render_modern`. Ошибки и прежняя offscreen bool semantics для существующего
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
| `config`, `suspended`, window sizing / resize handlers | R02b/c: размер drawable через `renderer_facade::drawable_size`, surface handover через команды фасада; resize outcome/`SurfaceConfiguration` остаются P01 |
| `device`, `queue`, `hud` uploads в skins/maps/UI/glint | A01/A02/E01/R02: CPU patches/resource commands; не протаскивать device/queue в общий trait |
| `screenshot*`, capture requests/results, device recovery | R02a: facade commands `request_screenshot`/`screenshot_pending`/`take_*`/`restore_*` уже используются Game; modern capture остаётся в modern adapter, legacy readback — U01 |
| `info`, gpu times/census, HDR calibration, shader macros/packs | Backend-neutral diagnostics; modern-only commands явно capability gated |
| `upload`, section membership/removals, `take_dropped_sections`, LOD / culling / static entities | W01/R02: neutral section keys/stamps, bounded queues, CPU visibility; modern GPU paths isolated |
| `skins_entities`, `skinned_models`, GPU posing storage | E01: настоящее CPU posing, не только `skin=None` |
| `gamma`, fullbright, animation/filtering, fog/lightmap | F01/A02: policy/settings commands, не публичные GPU-owned поля |
| `frame::Frame` и DTO `skin`/`lightning`/`pack` | R01a вынесены в общий CPU-модуль; старые `gfx::*` пути совместимы. Остаток R01: visibility/resource descriptors и отделение dependency graph, а не второй Frame |
| winit `Arc<Window>`, WindowEvent/KeyCode handlers | P01/P02/P03/P04/G01: platform adapters; `WakePolicy` уже вынесен, остальной loop ещё нет |

## 4. Что не замораживаем ложным обещанием

В этой ступени **ещё не реализованы** common menu request/static menu path, оставшиеся visibility/resource
descriptors и wgpu-free dependency graph, resource reload completion API, owning factory/renderer selection, screenshot result facade, полноценный
platform adapter и Game legacy startup. Их точные новые signatures согласует и внесёт интегратор;
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
