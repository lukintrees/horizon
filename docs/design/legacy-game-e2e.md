# Legacy Game E2E runner (T01a)

`tools/legacy_game_e2e.py` is a process/isolation harness, not a Game implementation or renderer test.
It creates fresh game, launcher, XDG data and config homes for each run, disables launcher self-update
and audio, captures stdout/stderr, enforces a timeout, and terminates only the child process group it
created. It refuses to reuse a nonempty `--home` unless the harness marker is present.

Example once the real Game startup wiring supports the event protocol:

```sh
python3 tools/legacy_game_e2e.py --timeout 180 --keep-home --result /tmp/run.json -- \
  cargo run --profile fast -p horizon-client -- --resources .local/horizon-client/26.3 --world e2e-test
# Reopen the same isolated test save:
python3 tools/legacy_game_e2e.py --timeout 180 --home /path/from/first/run --result /tmp/reopen.json -- \
  cargo run --profile fast -p horizon-client -- --resources .local/horizon-client/26.3 --world e2e-test
```

The child must emit exact machine-readable lines:

* `HORIZON_E2E_BACKEND legacy-gl` after selecting and initializing the requested legacy backend;
* `HORIZON_E2E_CHECKPOINT startup` after Game startup, `menu` after the actual menu is ready,
  `world` after a real world is loaded/render-ready, and `save` after the save operation completes.

At present, the central Game wiring does not provide this event protocol. These hooks are a request to
the lead integrator, not an invitation to add telemetry to Game in this task. Until then real Game runs
are blocked or fail; process exit zero and renderer fixtures are never sufficient evidence. The Python
unit tests use child-process stubs solely to validate harness behavior and are explicitly not proof that
the game launches. Future input/save assertions should be layered onto actual game-owned input and
persistence actions, with results recorded as named checkpoints/assertions in the run report; do not
simulate these by printing success lines in acceptance evidence.

Repeated runs use the same *marked test home* to reopen that test save. Never pass a user's home or
real save directory. A new run without `--home` uses a disposable fresh home and deletes it unless
`--keep-home` is given.
