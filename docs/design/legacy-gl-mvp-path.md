# Последовательный путь к MVP игры на native OpenGL 2

Дата: 2026-11-05. Цель устройства: Windows 7 SP1 x86 / Radeon X1650 / desktop GL 2.0–2.1.
Рабочая ветка: `arena/a7e96628-horizon`, integration draft PR #4.

Это **приоритет исполнения**, а не ещё один параллельный backlog. Он уточняет
[общий launch plan](legacy-gl-launch-plan.md); [правила интегратора](legacy-gl-integration.md)
и запрет менять default modern поведение остаются в силе.

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
resources здесь ещё не проверен, поэтому gate 1 **не закрыт**. Следующий blocker — visible
native window/event loop/loading menu; world CPU poses/visibility остаются впереди.


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
