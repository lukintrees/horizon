# Native Game bootstrap-only (MVP 1b)

Это промежуточный startup path в **существующем Game**, не playable MVP и не замена
будущему visible-window/menu gate. Флаг experimental и не часть Launch API 0.1.0.

## Кодовый маршрут

`Options::parse` → explicit `renderer=native-gl2` → validation → synchronous CPU resource load
→ `Game::new` → native owner from `pack.render` + `gui.atlas` → настоящий initial Title
→ `Game::hud` → `MenuFrame::Static` → existing native RendererFacade → один outcome → drop.

Нет winit event loop/window, `gfx::Renderer::new`, modern `PreparedTextures`, modern GPU skin
store, shader/resource packs или запуска внешних модов на этом route. Native профиль выбран
до UI/resource preparation: GUI atlas без декодирования panorama; static SDR menu. White
lightmap здесь намеренно menu-only, **не live world lighting**. World producers пока запрещены,
а не неявно урезаны до поддержанного subset. Native CLI не aliases wgpu `GraphicsApi::OpenGl`.

`--native-bootstrap-only` обязателен до подключения native event loop. Он рисует ровно один
кадр; `--frames`, если указан, допускает только `1`. World/server, control, benchmark,
headless, dev mods, session credentials, fullscreen и screenshot arguments отвергаются до
startup side effects. Native feature выключен — usage error, не modern fallback.

## Пробный запуск с собственными подготовленными ресурсами

Сначала собрать (полный client graph пока требует современный Rust/toolchain; это не Win7 artifact):

```sh
cargo build --locked -p horizon-client --no-default-features --features legacy-gl
```

Linux: нужны OSMesa/llvmpipe, normal prepared resource folder пользователя и **новые scratch folders**.
Никакие game assets не добавляются в Git и не скачиваются этим режимом.

```sh
# RESOURCE_DIR — абсолютный путь к вашим уже подготовленным ресурсам.
scratch=$(mktemp -d)
mkdir -p "$scratch/game" "$scratch/player" "$scratch/home"
HORIZON_HOME="$scratch/home" HORIZON_TELEMETRY_URL=off \
HORIZON_OSMESA_LIBRARY=libOSMesa.so.8 \
LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
MESA_GL_VERSION_OVERRIDE=2.0 MESA_GLSL_VERSION_OVERRIDE=120 \
./target/debug/horizon-client \
  --renderer native-gl2 --native-bootstrap-only --offscreen \
  --resources "$RESOURCE_DIR" --game-dir "$scratch/game" \
  --player-dir "$scratch/player" --name NativeBootstrap \
  --smoke-report "$scratch/bootstrap.json"
```

Успех этого сценария должен дать `scenario=native-game-bootstrap-only`, `backend=native-gl2`,
`menu_submissions=1`, `outcome=Offscreen`, `presented=false`, `playable=false`, `world_opened=false`.
Это завершённый offscreen draw, **не показанное окно**. Default native запуск Linux без
`--offscreen` сейчас явно отвергается до реализации visible window.

Windows: bootstrap выбирает существующий WGL owner и требует **отсутствия** `--offscreen`;
если выполнен реальный запуск, ожидается `Presented`. Это короткое окно без event loop,
не интерактивное меню и не Win7/X1650 qualification. Windows hardware здесь не проверялся.

Профиль не переписывает HDR/shader/resource-pack picks в saved options; native bootstrap не
запускает normal onGameLoadFinished settings-save/shut_down path. Shared resource loader всё
ещё может писать штатные derived caches, имя игрока и logs — поэтому только scratch paths.

## Evidence и открытый gate

В этом patch проверяются default/full-client compile, feature-enabled Game compile и actual
Options/profile tests, native factory CPU-negative и OSMesa GPU tests. Последние используют
renderer fixtures, **не полный Game resource load**. Полный запуск указанной команды с normal
prepared resources в текущем sandbox не выполнен: такого resource folder здесь нет.

Этап 1 остаётся открытым до наблюдаемого actual Game runtime evidence; профиль для world
CPU poses/visibility ещё не подключён. Следом по [MVP-плану](legacy-gl-mvp-path.md): native visible
Linux window, event adapter и настоящий loading/menu loop; не добавление новых renderer effects.
