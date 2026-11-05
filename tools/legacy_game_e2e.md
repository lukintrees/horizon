# T01a: isolated Game E2E runner

`tools/legacy_game_e2e.py` is the runner for the future real Game E2E.  It starts
the actual `horizon-client` process in a scratch home, observes the existing
control-socket and startup-trace checkpoints, and stops only the process group
it started.  It is deliberately not a renderer test and not a screenshot test.

## Run

```sh
python3 tools/legacy_game_e2e.py \
    --game target/fast/horizon-client \
    --resources .local/horizon-client/26.3 \
    --world T01a --seed 1234 --render-distance 6
```

Useful flags:

- `--offscreen`: render without a visible window (CI/headless sessions).  This is
  still a real Game run, but it is not a visible-window B1 qualification.
- `--timeout N`: startup and command timeout (default 180 s).
- `--out DIR`: report and artifact directory (default `target/legacy-game-e2e`).
- `--keep-home`: keep the scratch home after a passed run.
- `--home-root PATH --reuse-home`: reuse a scratch root and its test save.  The
  root must be a scratch path, never the real player home or game data folder.

Exit status: `0` passed, `1` failed, `2` blocked, `3` usage.

## Isolation

Every run gets a fresh root with separate `game/`, `home/`, `launcher/`,
`data/`, `config/`, `runtime/`, `artifacts/` and `tmp/` folders.  The runner
overrides `HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME`,
`XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR`, `HOME`, `USERPROFILE`, `APPDATA`,
`LOCALAPPDATA`, `TMPDIR`/`TEMP`/`TMP`, the launcher trash/clipboard/desktop
folders, and `HORIZON_STARTUP_TRACE`/`HORIZON_CONTROL`.  It uses `lab.game_env`,
so `HORIZON_NO_AUDIO`, `HORIZON_LAUNCHER_NO_SELF_UPDATE` and
`HORIZON_NO_DISCORD` are set.  The game's `saves/` and `options.txt` live under
the scratch `game/`; the real player's saves and settings are never read or
written.  A fresh `--home-root` must be empty; a reused one is kept.

The runner starts the game with `start_new_session=True` (POSIX) or
`CREATE_NEW_PROCESS_GROUP` (Windows).  Stopping uses `killpg` on the runner's own
group, or `taskkill /T /F` on the runner's own process tree.  It never calls
`pkill`, `killall` or another process's PID.

## Checkpoints

| Checkpoint | Existing observable | How the runner observes it |
|---|---|---|
| startup | control `hello`, `status`; `loading_screen.shown`/`title.shown` trace marks; log tail | connect, `hello`, `status`, trace |
| menu | `screen` returns `title`; `wait-for screen title`; `title.shown` | `screen`, `wait-for`, trace, screenshot |
| world | `wait-for loaded`; `state.player`; `status.chunks`; `world.first_frame`/`world.view_full` | `open`, `wait-for`, `state`, `status`, trace, screenshot |
| save | `save-all` acknowledgement; `worlds`; `saves/<world>/level.dat`; `quit` exit 0; reopen and read the block back | `setblock`, `save-all`, `worlds`, file check, `quit`, second run, `block` |

The save phase writes a stone block at the player's block position, runs
`save-all`, quits, starts a second process with the same scratch home, opens the
same world and reads the block back.  The second run must exit 0 and return
`minecraft:stone`; otherwise the save checkpoint fails.

A screenshot, a zero exit code or a renderer fixture is never accepted as proof.
The runner requires the real game binary (`--launch-api`), a control socket, the
four checkpoints and the backend confirmation below.

## Legacy backend confirmation (requested hook)

The existing `status.renderer.backend` is the wgpu backend (`Gl`, `Vulkan`,
`Dx12`, ...).  It is **not** a legacy GL2 confirmation.  The runner requires an
explicit machine-readable confirmation, proposed as:

```json
"legacy": {
  "confirmed": true,
  "backend": "legacy-gl",
  "gl_version": "2.1",
  "glsl_version": "1.20",
  "vendor": "...",
  "renderer": "..."
}
```

The `status.legacy` object is now published by Game, but the normal modern
startup correctly reports `confirmed=false` with empty GL fields. G01 still has
to populate it from a real native context. Missing hooks in older binaries or
unconfirmed legacy startup remain **blocked**, never successful. A wgpu `Gl`
backend is not native legacy confirmation.
An explicit wgpu backend (`Vulkan`, `Dx12`, `Metal`) or a log line mentioning a
wgpu fallback is an **erroneous fallback** and fails.  The runner never treats
"unavailable legacy startup" as success.

Integrator follow-up (not completed by the original T01a PR):

1. Fill the existing `status.legacy` hook from an actual native backend at G01;
   the current false/unconfirmed object is already integrated.
2. An explicit startup failure when legacy is requested but unavailable; no
   silent wgpu fallback.
3. A save acknowledgement that can be tied to a durable generation (optional;
   the current `save` acknowledgement plus `level.dat` and reopen is enough for
   T01a).

## Future input/save checks without changing Game

The runner already uses only existing control commands.  Future checks can be
added as scenario steps in `tools/legacy_game_e2e.py` without touching Game:

- input: `key`, `mouse`, `turn`, `click`, `type`, `click-button`, `click-slot`,
  `wait-for`, `input-log`, `events`;
- save: `inventory`, `block`, `save`, `save-all`, `worlds`, `open`, `state`,
  `world-backup`.

A future input probe should drive the same bindings a player uses (`key`,
`mouse`) and verify the result through `state`/`input-log`/`events`, not only
through a control-socket setter.  The runner's current scope deliberately does
not implement an input probe; that is a separate task.

## Unit tests and evidence

`tests/test_legacy_game_e2e.py` uses fake processes and a fake control client.
Those tests are **stubs**: they verify timeout, crash, missing checkpoints,
unconfirmed backend, erroneous fallback and the save-reopen state machine.  A
stub report has `"stub": true` and `"evidence_kind": "stub-control"`; it is not
evidence of a running game.  A real report has `"stub": false` and
`"evidence_kind": "game-process"`.

## Not verified here

- No real Game E2E was run in this task; the sandbox has no display/GPU and the
  legacy backend hook does not exist yet.
- No Win7 SP1 x86 / Radeon X1650 runtime qualification.
- No visible-window B1 check, input probe or full B2 scenario.
- No claim that a renderer fixture, screenshot or zero exit code is a Game E2E.
