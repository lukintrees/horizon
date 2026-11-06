# Последовательный путь к MVP игры на native OpenGL 2

Дата: 2026-11-06. Цель устройства: Windows 7 SP1 x86 / Radeon X1650 / desktop GL 2.0–2.1.
Рабочая ветка: `arena/a7e96628-horizon`, integration draft PR #4.

Это **приоритет исполнения**, а не ещё один параллельный backlog. Он уточняет
[общий launch plan](legacy-gl-launch-plan.md); [правила интегратора](legacy-gl-integration.md)
и запрет менять default modern поведение остаются в силе.

## Текущий прогресс — контрольная точка 2026-11-06

Предыдущий проверенный срез 3c закоммичен и отправлен в integration branch.
Его **кодовый** head: `de84398c74c2d5cb7b6b579fb3cf85800b0b90c4`.
`7a9c2deff` добавил native resident visibility/order и scope protection;
`de84398c7` исправил YAML запуска новой CPU-проверки. Это ещё **не playable MVP**.

| Участок | Что уже реализовано |
|---|---|
| MVP 1a/1b | Owning Modern/Native/Empty, opt-in `--renderer native-gl2`, CPU resource profile/factory; native не создаёт wgpu device/surface или GPU skin store |
| MVP 2a–2c | Visible GLX/WGL owner, native menu input, настоящее static-menu/HUD; окно до joined/cancellable resource worker, same-context checkpointed staging и atomic owner publication |
| MVP 3a/3b | Shared frustum, общие `Game::idle`/`frame_tick`, existing closing-save/progress routing; cadence 16/32 ms с минимумом 4 ms для save deadline, focus/lost-event reset |
| MVP 3c | `RendererOwner::native_visible_sections` и caller-owned `SectionOrder`: committed membership/Y origin, full 16³ AABB, f64 subtraction, solid nearest-first/key ties и обратный translucent order; pending GPU removal блокирует список |
| MVP 3d | Game world `Frame` caller → native resident visibility/facade; тот же camera frustum, Game tick animations, live lightmap/gamma/fullbright, existing CPU pose path; world-entry остаётся закрыт до streaming/input |
| Scope/CI | `renderer_owner/**` защищён от task edits; CPU default/native и actual-Frame/facade OSMesa regressions включены в focused matrix |

**Исторические проверки среза 3c:** [aggregate 42088555293](https://github.com/lukintrees/horizon/actions/runs/42088555293),
[PR 42088565354](https://github.com/lukintrees/horizon/actions/runs/42088565354) и
[PR после body refresh 42089916894](https://github.com/lukintrees/horizon/actions/runs/42089916894)
завершились SUCCESS, все семь jobs. Новая OSMesa fixture действительно исполнилась и прошла;
это renderer readback, не запуск normal-resource Game и не Win7 qualification.
Локально: 1308 Rust tests и 47 standalone render-api tests passed, Clippy/fmt прошли;
LF-normalized Python snapshot — 689 tests, 27 skipped, stable source hashes.
Windows `pr_ready --quick --base 1a12a2d51` остаётся **не green** только из-за unchanged
vendor-security symlink privilege error `WinError 1314` (689 Python tests, 15 skipped).
Crash source-location tests прошли с line tables; assertions/security tests не ослаблялись.

**Остаток:** native Game всё ещё menu/progress-only в normal startup. В 3d добавлен
Game world Frame caller через existing facade; runtime открытия мира не разрешён.
Stamped Game worker dispatch и upload/unload в `update_world`, затем gameplay input
ещё не подключены. World-entry,
options/reconnect/deferred-entry guards остаются до working native world submission;
controller polling отложен до adapter без fabricated winit IDs. Продолжать с seam'ов §3 ниже,
не повторять resident-order patch. Actual Game с normal prepared resources здесь не запускался:
runtime gates 1/2/3, полный save/reopen сценарий и Win7/X1650 остаются открытыми.

## Критерий MVP

**Настоящий существующий Game:** запуск → настоящее меню → локальный мир → ходьба/обзор →
выбор, ломание и установка блока → hotbar/инвентарь → сохранение → повторное открытие → выход.
Нельзя заменять Game отдельной мини-игрой, симуляцией или сценой из тестовых quads.

Два результата фиксируются отдельно:
- **Функциональный GL2 MVP** в видимом Linux окне с llvmpipe.
- **Целевой MVP**: тот же сценарий на Win7 SP1 x86/X1650 с аппаратным ICD и portable artifact.

Один только successful hosted Windows x86 build не закрывает второй результат.

## Что уже достаточно и не расширяется без blocker

На исходном `caed266` готовы renderer-level terrain/HUD/поддержанные Frame passes,
bounded producer snapshots, animation updates, same-context staged reload, glyph commit,
screenshot completion и stamped streaming control plane. Их проверки — регрессия, **не Game MVP**.
Не добавлять новые abstractions/effects/варианты очередей, если это не снимает конкретный
blocker ближайшего пункта ниже. Исправления ошибок и необходимая интеграция разрешены.

## 1. Выбор и ownership native renderer в Game — В РАБОТЕ

Последовательно:
1. Заменить конкретный `Game.renderer: Option<gfx::Renderer>` на owning storage Modern/Native/Empty,
   без Deref и без второго modern device на native пути. Современные accessors временно явно
   называются modern-only, чтобы места дальнейшей миграции не маскировались под общий API.
2. Отделить native выбор от `graphicsApi=opengl`: последнее — **wgpu OpenGL**, не native GL2.
   Ввести opt-in startup route; не fallback к modern при native ошибке.
3. Связать настоящий Game bootstrap с native factory из existing CPU producers. Native путь не
   вызывает `Renderer::new` modern, `PreparedTextures` и создание GPU skin store.
4. Выбрать минимальный legacy profile **до** генерации geometry: SDR, CPU poses, static menu,
   classic transparency, без shader packs/HDR/PBR/LOD/extras. Не обнулять запрошенные passes
   молча внутри renderer. Не сохранять временные overrides в modern settings.
5. Рано проверить Win7 toolchain/import/dependency риски. Наличие wgpu в промежуточной Linux
   сборке допускается, его инициализация native путём — нет; это не Win7 artifact.

**Выход:** настоящий Game владеет выбранным native owner, создаёт GL2 context без wgpu
initialization. Ownership refactor сам по себе этот пункт не закрывает.

Первый patch (MVP 1a): owning storage в поле Game и явные modern-only compatibility accesses.
Ранний аудит риска: на текущем срезе `horizon-client`/`horizon-client-render` требуют Rust 1.90,
`horizon-mods` — 1.95; wgpu/winit безусловны. Предыдущий
[Win7 audit](legacy-win7-build-audit.md) уже установил Tier-3 `i686-win7-windows-msvc` и необходимость
своего std; ordinary hosted `i686-pc-windows-msvc` нельзя выдавать за Win7 qualification.
Этот patch не уменьшает dependency closure и не меняет toolchain/CRT. Этот риск не задерживает Linux окно.

Продолжение MVP 1b: opt-in `--renderer native-gl2`, native client feature, static-menu CPU
resource profile и factory связаны с существующим `Game::new`/`Game::hud`. В том patch был
доступен только явно ограниченный `--native-bootstrap-only` (на Linux offscreen).
MVP 2a ниже добавляет visible menu-only route без этого flag, но не playable startup.
[Команда и ограничения](legacy-gl-native-bootstrap.md). Full Game runtime с normal prepared
resources здесь ещё не проверен, поэтому gate 1 **не закрыт**. Код native window/event loop/
loading menu добавлен в 2a–2c ниже; world Frame caller/CPU poses/visibility wiring
добавлены в 3d, но world entry/streaming/input runtime ещё не подключён.


## 2. Видимое окно, загрузка и настоящее меню — ОЖИДАЕТ 1

- Один Linux visible path (GLX/X11), не несколько display backends сразу; reuse WGL на Windows.
- Native loop/event adapter → существующие Game handlers, без поддельных winit IDs.
- Подключить actual loading/menu/HUD/glyphs; статический фон явно часть profile.
- Resize, close, pointer для UI, keyboard/text, focus/minimize и scheduling.

Промежуточный patch MVP 2a: добавлен один Linux GLX/X11 owner (реальное mapped window,
не pbuffer), native event pump и menu-only adapter в существующий Game/Title. Resize/focus/
close, pointer, базовые UI keys/Unicode text идут без поддельных winit IDs. World entry,
modern-only options и external actions пока показывают явную ошибку в настоящем Title;
симуляция не запускается. В этом первом patch CPU resources ещё загружались до окна;
следующий MVP 2b ниже меняет этот порядок. XIM/preedit/compose и gameplay input не закрыты.
Window-level Xvfb/llvmpipe evidence не заменяет actual Game runtime с normal resources.

Продолжение MVP 2b: native window + existing asset-free boot HUD теперь создаются **до**
resource worker. Один worker готовит existing resources + GameCpuResources, не создавая
modern PreparedTextures и detached rest loader. Main thread продолжает native pump/draw;
close/stop отменяют работу на CPU phase boundaries и ждут join с продолжающимся pump.
В первом срезе MVP 2b (`904da91`) GPU staging/handoff ещё был синхронным.

Продолжение MVP 2c: boot owner остаётся drawable во время staging candidate renderer на
**том же** Rc-owned Context/window. Checkpoints перед shader/backend creation, snapshot/layout,
перед первой и после каждой terrain page, на HUD/glint/animations/Ready помпят native events,
close/stop/timeout и перерисовывают existing boot HUD. Кандидат ограничен свободным tracked
GPU budget, дополнительно зарезервированы 2 MiB и mesh slots для временных boot HUD uploads.
Callback error удаляет partial candidate и boot без font publication; успех сначала удаляет
boot, затем возвращает общий budget/cache capacity, проверяет font epoch и одним move
публикует owner в Game. Это не global VRAM ledger и не preemptible GL: одна driver/CPU phase
остаётся синхронной, hard latency deadline не обещается. Offscreen/однокадровый bootstrap
сохраняет диагностический sync route.
Full assets runtime здесь всё ещё не наблюдался; окно/owner fixtures и compile не закрывают gate.
Reproducer: `tools/native_game_menu_smoke.py` с собственными normal prepared resources,
без control socket или synthetic Game. Это menu-only smoke, не world E2E/ручная навигация.

**Выход:** в видимом окне Game можно пройти меню и выбрать мир. Pbuffer/OSMesa не замена.
Полную main-menu panorama/blur не делать до playable world.

## 3. Настоящий мир и движение — ОЖИДАЕТ 2

- Existing worker dispatch получает ticket **до работы**, completions → готовая bounded queue.
- Existing CPU visibility/cave/frustum/order → native submission; не писать второй mesher.
- Live camera/lightmap/animation tick; CPU poses для обычных видимых объектов/руки.
- Existing gameplay bindings: WASD, jump, mouse-look/capture, Escape; simulation не переписывать.
- Ходьба должна вызывать загрузку новых и unload старых sections, edits/teleport не принимают stale jobs.

Начатый срез 3a: `Game::render` берёт размер кадра у `RendererOwner::size` и
строит camera frustum через `gfx::camera_frustum` — тот же код, которым
пользуется modern renderer. Это первый seam: у native owner нет wgpu device,
чтобы спросить frustum, а без него не построить CPU visibility ни для entities,
ни для sections. Поведение modern owner не изменилось.

Продолжение 3b: существующее тело winit `RedrawRequested` вынесено в общий
`Game::frame_tick`. Native pump теперь вызывает `Game::idle`, как headless loop;
он больше не ведёт отдельные frame counters/timeout и не обходит closing-save.
Close/Quit/stop запускают существующий save state machine, progress HUD идёт через
owner facade, а видимый native выход вызывает `Game::shut_down` и при ошибке pump.
Loading/file-fixer/reload HUDs используют тот же menu submission; modern сохраняет
panorama и offscreen completion, native явно выбирает static background.
Размер drawable теперь читается через `&self`, без отдельного кэша размеров/окна.

Native cadence ограничена 16 ms при activity и 32 ms без focus/при minimize,
включая очередь pending OS messages; короткий save deadline может снизить интервал
до 4 ms, но не разрешает busy-spin. Focus/minimize/lost-event сбрасывают held UI
input. Native controller polling пока отключён: его menu adapter ещё генерирует
winit events с synthetic IDs. World entry/options/reconnect/deferred requests
по-прежнему отказывают в общем Game до неподключённых side effects. Это **seam,
не runtime acceptance**: normal-resource Game и мир здесь ещё не запускались.

Промежуточный срез 3c: `RendererOwner::native_visible_sections` строит
caller-owned `SectionOrder` из committed native residency и её Y origin.
Shared camera frustum / distance predicate, полные 16³ AABB и f64 subtraction
дают solid nearest-first/key ties и обратный translucent order. Pending GPU
removal запрещает публикацию списка. Это ещё не Game caller: world-entry guard
остаётся, а synthetic actual-Frame readback не закрывает runtime gate.

Продолжение 3d: `Game::render` сохраняет отдельный native Title/progress путь,
но world frame строится существующим producer и подаётся через
`RendererOwner::render_native_world`. Caller-owned order использует committed residency,
shared camera frustum и radius. Animation tick берётся из `Frame.ticks` с опубликованной
resource generation; live lightmap, gamma/fullbright, sky/fog и CPU geometry проходят
existing facade/coordinator. `near_plane` тоже получает aspect через owner, не wgpu.
Empty/non-native owner, title panorama и неинициализированный world streaming — ошибки,
а не fallback/поддельный empty world. Ошибки не обходят восстановление Game frame buffers;
ошибочный visibility scratch очищается, minimized drawable откладывается. Modern route
сохранён. CPU skinning следует уже существующему `modern_ref`-gated GPU/CPU выбору;
запрошенный GPU SkinFrame явно отвергается native coordinator, не теряется молча.

Новая OSMesa regression подаёт реальный Frame/SectionMesh contract: terrain camera/order,
clock-driven red/blue animation, изменяемый lightmap/gamma/fullbright, CPU entity quad,
refusal GPU poses/panorama/invalid camera с сохранением предыдущего framebuffer.
Это fixture и compile wiring, **не actual normal-resource Game world**. Cave graph
modern owner в этот conservative frustum/distance walk не перенесён.

Следующие seam'ы по порядку:
1. Streaming: stamped dispatch/completions и upload/unload в `update_world` через facade
   для legacy owner (сейчас весь блок под `modern_mut()`); tickets W01a уже в ветке.
   Снять world-entry guard только вместе с working Game world residency/input, не ради
   sky-only кадра. Native world submission требует initialized streaming identity.
2. Gameplay input (WASD/jump/capture/Escape) на native pump.
3. Actual normal-resource Game runtime и оставшиеся конкретные producers сценарием §3/4.

Exact-head проверки 3d публикуются в draft PR #4 отдельно от исторических результатов 3c.

**Выход:** игрок ходит по настоящему миру; chunks появляются и выгружаются в движении.
Только конкретные unsupported producers этого сценария становятся blockers следующего patch.

## 4. Играбельность основного сценария — ОЖИДАЕТ 3

- Block outline, breaking/crumbling overlay.
- Обычная рука/предмет, hotbar, инвентарь и стандартные entities/материалы, встреченные в сценарии.
- Исправлять реальные blockers, не расширять полный каталог VFX «на всякий случай».

**Выход:** выбрать, сломать и установить блок; открыть инвентарь. Важная geometry не скрывается.

## 5. Сохранение и повторный вход — ОЖИДАЕТ 4

- Actual Game save/close/reopen; pause/focus/minimize не оставляют held input.
- Renderer transaction связать с CPU pack/language publication и отменой worker generations.
- Ограниченный повторный world entry/reload цикл с измерением tracked residency.

**Выход:** изменённый мир сохраняется и открывается заново; нет потери данных/растущих очередей.
Это первый полный функциональный MVP на Linux/llvmpipe, не hardware qualification.

## 6. Целевой Win7 x86 artifact — ОЖИДАЕТ 5; риски проверяются с 1

- Замкнуть legacy dependency graph; PE32, std/toolchain, CRT, imports и ISA.
- Portable files/resources/run instructions без чужих game assets в Git.
- Выполнить тот же маршрут на Win7/X1650: input, Alt-Tab, resize, save/reopen; зафиксировать
  driver/GL limits, ошибки и измерения. Нет стенда — gate остаётся открытым, не заменяется CI.

**Выход:** целевой artifact и реальный отчёт запуска на устройстве.

## Правила продвижения

- Идти 1 → 2 → 3 → 4 → 5 → 6; не открывать новый workstream вместо ближайшего runtime blocker.
- Каждый runtime этап заканчивается наблюдаемым действием в **Game**. Compile/unit/GPU tests —
  необходимые регрессии, не замена acceptance. Допустимы промежуточные refactor patches, статус честный.
- Default modern compile/Game tests после общей правки; native checks/x86 по изменённому коду.
- Прогоны только в fresh scratch HORIZON_HOME/game/config/data folders; не трогать реальные saves.
- Никаких B1–B4/playable/Win7 claims до соответствующего runtime evidence. Main merge/release не
  разрешены этим планом. Изменения shared contracts/manifests делает только текущий интегратор.
