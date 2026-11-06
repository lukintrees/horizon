# Последовательный путь к MVP игры на native OpenGL 2

Дата: 2026-11-06. Цель устройства: Windows 7 SP1 x86 / Radeon X1650 / desktop GL 2.0–2.1.
Рабочая ветка: `arena/a7e96628-horizon`, integration draft PR #4.

Это **приоритет исполнения**, а не ещё один параллельный backlog. Он уточняет
[общий launch plan](legacy-gl-launch-plan.md); [правила интегратора](legacy-gl-integration.md)
и запрет менять default modern поведение остаются в силе.

## Текущий прогресс — контрольная точка 2026-11-06

Проверенный baseline 3e: `2bd8d3b1e523b04047607affab1284e681a8eb36`
([evidence](https://github.com/lukintrees/horizon/pull/4#issuecomment-6683256849)).
Текущий кодовый срез добавляет shared native gameplay input (§Native input ниже);
его exact-head проверки фиксируются отдельным комментарием в draft PR #4.
Это ещё **не playable MVP**.

| Участок | Что уже реализовано |
|---|---|
| MVP 1a/1b | Owning Modern/Native/Empty, opt-in `--renderer native-gl2`, CPU resource profile/factory; native не создаёт wgpu device/surface или GPU skin store |
| MVP 2a–2c | Visible GLX/WGL owner, native menu input, настоящее static-menu/HUD; окно до joined/cancellable resource worker, same-context checkpointed staging и atomic owner publication |
| MVP 3a/3b | Shared frustum, общие `Game::idle`/`frame_tick`, existing closing-save/progress routing; cadence 16/32 ms с минимумом 4 ms для save deadline, focus/lost-event reset |
| MVP 3c | `RendererOwner::native_visible_sections` и caller-owned `SectionOrder`: committed membership/Y origin, full 16³ AABB, f64 subtraction, solid nearest-first/key ties и обратный translucent order; pending GPU removal блокирует список |
| MVP 3d | Game world `Frame` caller → native resident visibility/facade; тот же camera frustum, Game tick animations, live lightmap/gamma/fullbright, existing CPU pose path; world-entry остаётся закрыт до streaming/input |
| MVP 3e | Existing Game mesher получает owner-issued stamp до dispatch; bounded native upload/unload и admission; modern API/default сохранён. Membership/view/epoch invalidation и explicit capacity/oversize errors; world-entry по-прежнему закрыт |
| Native input | Game-owned CPU input DTO, общие Game handlers/MouseHandler; XKB/Win32 physical HID, XI2/WGL raw relative capture, focus/keymap/drop reset. Normal-resource gameplay и WGL runtime ещё не квалифицированы |
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
В 3e подключены stamped Game worker dispatch и upload/unload в `update_world`.
Gameplay input ещё не подключён. Проверки 3e — CPU protocol и direct native-hook OSMesa;
это не real-worker/normal-resource end-to-end запуск Game. World-entry,
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

## MVP 3e — stamped dispatch и bounded native delivery

`streaming/admission.rs` расширяет existing CPU scheduler, не создаёт новый worker pool.
До `Job::Mesh` native owner выдаёт `SectionStamp`; `MeshSettings` → `Done::Meshed`
переносят его неизменным. Existing latest-sequence/loaded-column checks выполняются
до native sink. Backpressure сохраняет тот же mesh/stamp/sequence в existing deferred
set; admission defer возвращает dirty bit. Payload передаётся через `Arc`, без clone
vertex vectors. Modern `Streaming::update` оставляет прежний upload-list API.

`renderer_owner/native_streaming.rs` принадлежит Game и owner thread. Rebind CPU
streamer identity/layout/resource generation очищает native epoch и redirties loaded
CPU sections. `ChunkMap::deltas_since` отзывает tickets даже при remove+insert тех же
координат; потеря ring history/замена map консервативно отзывает все интересы. Выход
из draw radius отзывает GPU resident и redirties ещё загруженную CPU column для
возвращения. Возвращённые unloaded modified chunks идут в общий Game entity/save
cleanup даже при последующей GPU ошибке; ошибка прерывает frame до spawn/render.

Bounded defaults: 4096 section interests, 64 pending, 8 MiB retained payload;
обычный drain — 4 jobs/8 MiB за update (configured jobs capped at 64). Mesh больше
queue/per-drain byte capacity — явная ошибка, не вечная голова очереди. Admission
также проверяет, что **все sections** полного CPU draw radius помещаются в slots:
permanent metadata starvation нельзя выдавать за transient backpressure. Радиус
не clamp'ится молча; неподдерживаемый view получает ошибку. Для vanilla 24 sections
и 4096 slots CPU test принимает radius 6 и отклоняет 7. Это metadata bound, **не**
обещание общего VRAM бюджета/жёсткой длительности GL upload.

Проверки: actual production CPU delivery helper фильтрует stale sequence/unloaded
column до sink и сохраняет retry stamp. Direct-hook OSMesa fixtures проверяют actual
native queue/residency: pending backpressure, air replacement, revoke/unload, old
world ticket, oversize error, view reentry, same-coordinate reload, map replacement,
lost delta history и epoch redirty. Эти fixtures **не запускают реальный worker** и
не доказывают normal-resource Game/gameplay. Entry/input/save-reopen/hardware gates
остаются открыты; следующий блок — native gameplay input и real-worker integration
перед снятием world-entry guards.


## Native input — общий Game путь, до real-worker acceptance

`game/input_event.rs` отделяет CPU payload от winit producer: modern сохраняет
реальные window/device identity, layout learning и raw mod hook, а затем преобразует
payload. Native передаёт physical HID, Unicode, кнопки, колесо и resize/focus в те же
`Game::handle_input_event`/screen handlers. Нет изготовленных WindowId/DeviceId,
приватного winit KeyEvent или отдельной реализации movement/inventory. Native
logical key пока явно `Unidentified`; locale-specific logical shortcuts и native
mod input ABI не квалифицированы. Normal native bootstrap по-прежнему не запускает mods.

Обзор использует общий `Game::relative_motion`/MouseHandler с прежними sensitivity,
smoothing, GUI gates и finite check, но native delta производится настоящим
XI2 RawMotion / Win7 RawInput. Это не разность ограниченной позиции курсора и не
recenter loop. Capture требует активного окна; release/focus/minimize/keymap reset
сбрасывают удержания, modifiers, drag, mouse accumulator и text-opening gate.
Символ открывающей чат клавиши не попадает в только что открытое поле.

На GLX libXi удерживается до XCloseDisplay, включая отказ настройки capture:
расширение регистрирует callback в Display. В интеграционном тесте libXtst также
живёт дольше Display даже при panic, иначе callback исполняется после dlclose и
маскирует реальную причину assertion через SIGSEGV. XI2 version negotiation
объявляет 2.2, сохраняя использование только RawMotion полей 2.0.

Real-X-server regression использует XTest physical W press/release и три движения
(+1024, −1024) при размере окна 64×64: требует суммарный raw delta не меньше
(+3072, −3072), затем проверяет free mode, focus loss, отказ захвата чужого focus,
закрытие с активным захватом и повторный захват новым owner. ASSERT не заменён
ограниченной позицией курсора. Fmt/Clippy, default modern/native CPU game и x86
Windows compile входят в branch CI; exact-head результаты фиксируются в PR, а не
считаются пройденными по наличию теста. Windows compile не доказывает WGL runtime.

World-entry guard и minimal menu-only policy сохранены (включая fullscreen shortcut
до записи settings). Нормальный prepared-resource Game, настоящий worker в
acceptance-сценарии, walk/look/break/place/inventory/save/reopen и Win7/X1650 ещё
не проверены. Этот срез не снимает ни один из этих runtime gates.


## Real mesher worker — подготовленный regression, runtime BLOCKED без ресурсов

Добавлен opt-in `renderer_owner::native_streaming::worker_tests::prepared_resources_real_mesher_rejects_inflight_replaced_columns`.
Он загружает настоящие prepared `Assets`/models/sprites/facts/biomes/occlusion,
запускает существующий shared jobs pool и Mesher через `Streaming` и тот же
`NativeStream::update`, который вызывает Game. Mesh и SectionStamp не подставляются.
Колонки подаются через existing remote-column API: это изолированная проверка
mesher delivery, **не vanilla terrain generation, не Game/world/save E2E**.
Boot HUD не используется для проверки world pixels. Test-only counters требуют
реального terminal outcome старого job: retirement ещё queued snapshot либо actual
`Delivery::Obsolete` для уже начавшейся работы. Одного завершения до возврата старого
job недостаточно.

Сценарий требует: реальный stone mesh в GPU/readback → dispatch старого snapshot →
forget/reinsert того же coordinate с gold block до сбора completion → новый issued
ticket и отличающийся readback → forget с чёрным framebuffer. Native pending queue
ограничена одним payload/drain; текущий meshing set — 32 sections для одного worker.
Текстуры stone/gold в normal pack должны различаться. При провале нет фиктивного
успешного отчёта; ignored/zero matching tests не считаются исполнением.

При подготовке этого сценария обнаружен конкретный native admission blocker:
старый `room = workers * 32 - meshing.len()` ограничивал **число columns**, после
чего dispatch отправлял все их sections. Native-only `DispatchQuota` теперь
проверяет section room **до** выдачи ticket/sequence, а Defer не расходует quota и
оставляет section dirty. Modern admission сохраняет прежний путь/default behavior.
Это ограничение текущего accepted meshing set, **не** гарантия общего CPU heap,
отмены уже исполняющегося старого job или hard frame latency.

Сборка/запуск на Linux с OSMesa и подготовленными ресурсами:

```sh
cargo test --locked -p horizon-client-render --no-default-features --features legacy-gl --lib --no-run
# --tests: путь к test executable, напечатанному Cargo (не Game binary).
python3 tools/native_worker_smoke.py --tests "$RENDER_TEST_EXECUTABLE" \
  --resources "$RESOURCE_DIR" --report .local/legacy-gl/native-worker.json
```

Runner запускает ровно этот ignored test в отдельном temporary home, принудительно
на Mesa GL 2.0/GLSL 120, сохраняет binary/manifest SHA256, реальные counters и
результат assertions. Нет загрузки assets, автогенерации resource fixtures или
control socket. Missing prerequisites дают exit 2 / `blocked`, а не PASS.

В текущем workspace prepared `horizon-resources.json` не найден. Поэтому этот
**real-worker runtime ещё НЕ исполнен**, даже если его компиляция/CPU quota/runner
unit tests прошли CI. Для исполнения нужен normal prepared resource directory.
World-entry guards и все acceptance gates из предыдущего раздела остаются закрыты.
Fullscreen/F2 в минимальном menu profile отказывают до записи settings/screenshots.


## Native queued mesh retirement — CI/local Rust без prepared resources

Native `update_admitted` до нового dispatch удаляет из actual job heap только
`Job::Mesh`, если sequence уже не current или колонка отсутствует. `forget_columns`
раньше очищал meshing map, но оставлял queued Arc region snapshots до будущего pop
worker'ом: повторные перемещения могли удерживать старые колонки вне текущей quota.
Новый retirement не меняет modern scheduling (тот же opt-in admission flag), не
удаляет Save/Free/Light/Generate и сохраняет priority/order оставшейся работы.

Payloads возвращаются из helper и уничтожаются **после** освобождения queue mutex;
fast path без obsolete jobs сохраняет allocation heap. Работа, уже взятая worker'ом,
не считается canceled и проходит прежний sequence/column gate при completion.
Это не preemption CPU calls, общий heap budget или hard frame latency guarantee.

Ресурс-независимые проверки:

- Actual `streaming/retirement.rs` исполняется напрямую локальным rustc через
  `python3 -m unittest discover -s tests -p test_streaming_retirement.py`: сохранение
  priority/order, fast-path allocation и отсутствие premature payload destruction.
- CI исполняет actual `Queued`/`Job::Mesh` tests: superseded sequence, unloaded column,
  сохранение Save/Free/Light/Generate, 64 repeated forget cycles с Weak-проверкой
  освобождения snapshots, а также реальный scoped thread → actual Mailbox → actual
  `deliver_current` после forget. Последний использует **mesh transport fixture**,
  не resource-dependent Mesher, и не считается normal-resource worker acceptance.
- Prepared-resource runner различает retired queued jobs и obsolete completions;
  оба нуля — провал. Это не позволяет принять старую работу, ещё не получившую
  terminal outcome, за успешно проверенную отмену.

Prepared-resource scenario по-прежнему не исполнялся; world/save/Win7 gates не сняты.


## Resource epoch/backpressure churn и world-exit queue lock — CI/local Rust

Следующий ресурс-независимый блок не расширяет renderer feature scope:

- `abort_generation` сохраняет прежнее правило «после abort остаются только Saves»,
  но возвращает ownership canceled jobs из heap helper, отпускает queue mutex и
  будит pool **до** освобождения payloads. Раньше последние Arc region/mesh/proto
  references уничтожались внутри queue lock и мешали worker'ам забрать pending Save.
  Это исправление lock lifetime, не новая save реализация и не save/reopen acceptance.
  Actual Queued/Job test проверяет сохранение opaque Save payloads/FIFO и pop реальным
  scoped worker'ом, пока canceled snapshots ещё удерживаются owner'ом.
- Independent render-api Rust test исполняет 20 000 owner-state operations через
  production `Stream`: fresh/superseded requests, stale/duplicate completions,
  backpressure/retry, out-of-order/duplicate ack, cleanup retry, resource rebase.
  Usage сверяется с actual pending storage; Weak проверяет отсутствие удержания
  rejected/retired payloads. Coverage counters не дают пустому состоянию выдать
  vacuous pass. Это generic ticket/ownership contract, не процессный RAM budget.
- Отдельный opt-in OSMesa test проходит actual native Hook + staged resource reload:
  `pending=1`, CPU Retry без копирования Vec, failed/canceled stage сохраняет old
  resident pixels/pending/ticket, committed stage очищает old pending/residency и
  отвергает старый CPU stamp даже до rebind. После rebind fresh owner ticket рисует
  green вместо red; повторный bind той же эпохи не сбрасывает current residency.
  CI явно исполняет `renderer_owner::native_streaming::gpu_epoch_tests:: --ignored`.
  Это GL2 readback с transport geometry/test assets, не normal-resource Mesher/Game.

Default modern scheduling, renderer contracts и world-entry guard не изменены.
Normal-resource/Game/save-reopen и Win7/X1650 gates остаются OPEN.


## Native consumer interest до dispatch — forced-column starvation regression

Найден конкретный scheduler/owner mismatch: `Ranges::in_view` принимает forced
колонки (это корректно для generation/modern), но native `Hook::prune` держит только
текущий draw view. Раньше Hook мог выдать ticket такой off-view колонке, запустить
Mesher и сразу отозвать ticket на post-update prune. При большом forced/extra наборе
такие ready candidates с высоким priority также вытесняли текущий view из
`nearest_first` и тратили native section quota на гарантированно stale работу.

Исправление не меняет generation/forced/save policy и default modern behavior:

- Default `MeshAdmission::wants_column` принимает все колонки. Native Hook использует
  **тот же view predicate**, что prune; bounds явно заданы при создании Hook и
  обновляются prune. Rejected columns не mint tickets, даже когда metadata slot свободен.
- Existing unloaded-dirty cleanup и обычный out-of-range waiting остаются **до**
  consumer filtering. Ready candidates фильтруются in-place **до nearest-first** и
  до cloning region snapshots/dispatch. Неиспользованные forced masks сохраняются.
- Native queued retirement использует тот же interest predicate: уже queued,
  current-but-off-view Mesh jobs отпускают snapshots и current-map quota; running
  CPU jobs не preempted и проходят прежний completion gate.

Resource-free actual Queued/DirtySections/SectionMask regression создаёт 128 forced
колонок, каждая с priority ниже wanted колонки, и room=1. Проверяется baseline
starvation, 256 повторений корректного scoped отбора, modern preservation, сохранение
mask bits через все четыре u64 слова, retirement текущего off-view queued job и
readmission сохранённой dirty колонки после смены scope.

Отдельный opt-in native Hook/OSMesa regression делает 1024 off-view requests при
**свободном metadata slot** и pending=1, проверяет неизменные usage/tickets и отсутствие
hidden nonce minting. Затем реальная смена view отзывает pending/retry, разрешает
fresh forced-column ticket, radius=0 очищает interests, возврат view использует новый
stamp и не принимает старый. CI требует named pass этой regression, а не exit zero
с нулём matched tests. Existing epoch rollback/commit readback остаётся в матрице.

Эти проверки — scheduler/owner/GL2 fixtures, не normal-resource Mesher/Game. Prepared
worker scenario и Game/save/reopen/Win7-X1650 gates остаются OPEN; world-entry guard
не изменён. Общий heap budget и preemption/hard frame latency не заявляются.
