#!/usr/bin/env python3
"""Isolated runner for the future real Game E2E (T01a).

Starts the real horizon-client in a scratch home, observes startup, menu, world
and save checkpoints through the existing control socket and startup trace, and
stops only the process group it started.  A renderer fixture, a bare exit code
or a screenshot is never accepted as a Game E2E.

status.legacy is the confirmation hook requested from the integrator.  Until the
game reports it, a legacy run is blocked, not passed.  See the short guide.

    python3 tools/legacy_game_e2e.py --game target/fast/horizon-client \
        --resources .local/horizon-client/26.3 --world T01a --seed 1234

Exit status: 0 passed, 1 failed, 2 blocked, 3 usage.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

try:
    from tools import lab
except ImportError:  # running as a script: tools/ is on sys.path
    import lab

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = 1
TASK_ID = "T01a"
CHECKPOINTS = ("startup", "menu", "world", "save")
LEGACY_BACKEND = "legacy-gl"
REQUIRED_LEGACY_FIELDS = ("backend", "gl_version", "glsl_version", "vendor", "renderer")


class RunnerError(Exception):
    def __init__(self, message: str, blocked: bool = False):
        super().__init__(message)
        self.blocked = blocked


def last_json(text: str) -> dict | None:
    for line in reversed(text.splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    return None


def tail_text(path: Path, limit: int = 120) -> str:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""
    return "\n".join(text.splitlines()[-limit:])


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def read_trace_marks(path: Path) -> dict[str, int]:
    marks: dict[str, int] = {}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return marks
    for line in text.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and event.get("ph") == "i" and event.get("name"):
            marks.setdefault(str(event["name"]), int(event.get("ts", 0)))
    return marks


def newest_crash_report(home: Path) -> Path | None:
    folder = home / "crash-reports"
    if not folder.is_dir():
        return None
    reports = [p for p in folder.iterdir() if p.is_file()]
    return max(reports, key=lambda p: p.stat().st_mtime) if reports else None


def checkpoint(name: str, observed: bool = False, evidence: dict | None = None,
               missing: str | None = None) -> dict:
    return {"name": name, "observed": observed, "evidence": evidence or {}, "missing": missing}


def backend_evidence(confirmed: bool, state: str, reason: str, evidence: dict) -> dict:
    return {"confirmed": confirmed, "state": state, "reason": reason, "evidence": evidence}


def inspect_backend(status: dict, log_text: str = "") -> dict:
    """Return the requested status.legacy confirmation, or why it is unavailable.

    wgpu's status.renderer.backend is not a legacy confirmation: wgpu Gl is the
    modern path, and an explicit wgpu backend is an erroneous fallback.
    """
    legacy = status.get("legacy")
    renderer = status.get("renderer") if isinstance(status.get("renderer"), dict) else {}
    backend = renderer.get("backend")
    evidence = {"status": status, "log_tail": log_text[-4000:]}
    if isinstance(legacy, dict) and legacy.get("confirmed") is True:
        evidence["legacy"] = legacy
        if legacy.get("backend") not in (LEGACY_BACKEND, "LegacyGl", "legacy-gl2"):
            return backend_evidence(False, "incomplete",
                                    f"legacy confirmation names backend {legacy.get('backend')!r}", evidence)
        missing = [key for key in REQUIRED_LEGACY_FIELDS if not legacy.get(key)]
        if missing:
            return backend_evidence(False, "incomplete",
                                    "legacy confirmation missing " + ", ".join(missing), evidence)
        if backend in {"Vulkan", "Dx12", "Metal"}:
            return backend_evidence(False, "fallback",
                                    f"legacy confirmation contradicts wgpu backend {backend}", evidence)
        return backend_evidence(True, "confirmed", "legacy backend confirmed by status.legacy", evidence)
    if isinstance(legacy, dict):
        evidence["legacy"] = legacy
        if legacy.get("confirmed") is False:
            return backend_evidence(False, "unconfirmed", "status.legacy reports confirmed=false", evidence)
    if backend in {"Vulkan", "Dx12", "Metal"}:
        return backend_evidence(False, "fallback",
                                f"modern wgpu backend {backend} used; legacy was requested", evidence)
    if backend == "Gl":
        return backend_evidence(False, "unconfirmed",
                                "wgpu GL backend, not the legacy GL2 renderer; "
                                "status.legacy confirmation hook is missing", evidence)
    if backend == "LegacyGl":
        return backend_evidence(False, "unconfirmed",
                                "renderer reports LegacyGl but status.legacy confirmation is missing", evidence)
    lowered = log_text.lower()
    if "wgpu" in lowered and "fallback" in lowered:
        return backend_evidence(False, "fallback", "log mentions a wgpu fallback", evidence)
    return backend_evidence(False, "unconfirmed",
                            "legacy backend not confirmed; status.legacy hook is missing", evidence)


def assert_scratch_root(root: Path) -> None:
    root = root.resolve()
    home = Path.home().resolve()
    if root == home:
        raise RunnerError("refusing to use the real home as a test root", blocked=True)
    real = [home / ".local/share/horizon", home / ".local/share/horizon-launcher"]
    if os.name == "nt":
        appdata = os.environ.get("APPDATA")
        if appdata:
            real += [Path(appdata) / "horizon", Path(appdata) / "horizon-launcher"]
    for path in real:
        if root == path or path in root.parents:
            raise RunnerError(f"refusing to use real player data path {path}", blocked=True)


def default_launch_api(game: Path, env: dict[str, str], cwd: Path) -> dict:
    result = subprocess.run([str(game), "--launch-api"], cwd=str(cwd), env=env,
                            capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise RuntimeError(f"exit {result.returncode}: {result.stderr.strip()[:500]}")
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"not JSON: {error}") from error
    if not isinstance(data, dict) or "launch_api" not in data:
        raise RuntimeError("not a Horizon Launch API reply")
    return data


def find_ctl(game: Path, explicit: Path | None) -> Path:
    if explicit is not None:
        return explicit
    for candidate in (game.parent / "horizon-ctl", game.parent / "horizon-ctl.exe",
                      ROOT / "target/fast/horizon-ctl", ROOT / "target/release/horizon-ctl"):
        if candidate.is_file():
            return candidate
    found = shutil.which("horizon-ctl")
    return Path(found) if found else Path("horizon-ctl")


class TestHome:
    def __init__(self, root: Path, reused: bool):
        self.root = root.resolve()
        self.reused = reused
        if not reused and self.root.exists():
            if not self.root.is_dir() or any(self.root.iterdir()):
                raise RunnerError("home-root exists and is not empty; use --reuse-home or a fresh path",
                                  blocked=True)
        self.game_dir = self.root / "game"
        self.home = self.root / "home"
        self.launcher_home = self.root / "launcher"
        self.data_home = self.root / "data"
        self.config_home = self.root / "config"
        self.runtime_dir = self.root / "runtime"
        self.artifacts = self.root / "artifacts"
        for folder in ("game", "home", "launcher", "data", "config", "runtime", "artifacts",
                       "trash", "clipboard", "desktop", "tmp"):
            (self.root / folder).mkdir(parents=True, exist_ok=True)

    @classmethod
    def create(cls, root: Path, reused: bool) -> "TestHome":
        return cls(root, reused)

    def env(self, trace: Path, socket: Path) -> dict[str, str]:
        return lab.game_env({
            "HORIZON_HOME": str(self.home), "HORIZON_LAUNCHER_HOME": str(self.launcher_home),
            "XDG_DATA_HOME": str(self.data_home), "XDG_CONFIG_HOME": str(self.config_home),
            "XDG_RUNTIME_DIR": str(self.runtime_dir),
            "HORIZON_LAUNCHER_TRASH": str(self.root / "trash"),
            "HORIZON_LAUNCHER_CLIPBOARD": str(self.root / "clipboard"),
            "HORIZON_LAUNCHER_DESKTOP": str(self.root / "desktop"),
            "HORIZON_TELEMETRY_URL": "off", "HORIZON_STARTUP_TRACE": str(trace),
            "HORIZON_CONTROL": str(socket), "HOME": str(self.home), "USERPROFILE": str(self.home),
            "APPDATA": str(self.data_home), "LOCALAPPDATA": str(self.data_home),
            "TMPDIR": str(self.root / "tmp"), "TEMP": str(self.root / "tmp"), "TMP": str(self.root / "tmp"),
        })


class LocalProcess:
    def __init__(self, command: list[str], cwd: Path, env: dict[str, str],
                 stdout_path: Path, stderr_path: Path):
        self.command, self.pid, self.pgid = command, 0, None
        stdout_path.parent.mkdir(parents=True, exist_ok=True)
        stderr_path.parent.mkdir(parents=True, exist_ok=True)
        self._stdout, self._stderr = stdout_path.open("wb"), stderr_path.open("wb")
        options: dict = {"cwd": str(cwd), "env": env, "stdin": subprocess.DEVNULL,
                         "stdout": self._stdout, "stderr": self._stderr}
        if os.name == "nt":
            options["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        else:
            options["start_new_session"] = True
        try:
            self.proc = subprocess.Popen(command, **options)
        except OSError:
            self._stdout.close()
            self._stderr.close()
            raise
        self.pid = self.proc.pid
        if os.name != "nt":
            try:
                self.pgid = os.getpgid(self.pid)
            except OSError:
                self.pgid = None

    def poll(self) -> int | None:
        return self.proc.poll()

    def wait(self, timeout: float) -> int | None:
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            return None

    def stop(self) -> None:
        if self.proc.poll() is None:
            if os.name == "nt":
                taskkill = shutil.which("taskkill")
                if taskkill:
                    subprocess.run([taskkill, "/PID", str(self.pid), "/T", "/F"],
                                   capture_output=True, check=False)
                else:
                    self.proc.terminate()
            else:
                try:
                    group = os.getpgid(self.pid)
                except OSError:
                    group = None
                if group is not None and group == self.pid:
                    try:
                        os.killpg(group, signal.SIGTERM)
                    except OSError:
                        pass
                else:
                    try:
                        os.kill(self.pid, signal.SIGTERM)
                    except OSError:
                        pass
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                if os.name == "nt":
                    taskkill = shutil.which("taskkill")
                    if taskkill:
                        subprocess.run([taskkill, "/PID", str(self.pid), "/T", "/F"],
                                       capture_output=True, check=False)
                    else:
                        self.proc.kill()
                elif self.pgid is not None and self.pgid == self.pid:
                    try:
                        os.killpg(self.pgid, signal.SIGKILL)
                    except OSError:
                        pass
                else:
                    try:
                        os.kill(self.pid, signal.SIGKILL)
                    except OSError:
                        pass
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
        self._stdout.close()
        self._stderr.close()


class HorizonCtl:
    def __init__(self, ctl: Path, socket: Path, env: dict[str, str], cwd: Path, timeout: float = 30.0):
        self.ctl, self.socket, self.env, self.cwd, self.timeout = ctl, socket, env, cwd, timeout

    def command(self, name: str, *args: str, timeout: float | None = None) -> dict:
        command = [str(self.ctl), "--socket", str(self.socket), name, *args]
        try:
            result = subprocess.run(command, cwd=str(self.cwd), env=self.env, capture_output=True,
                                    text=True, timeout=self.timeout if timeout is None else timeout)
        except subprocess.TimeoutExpired:
            return {"error": f"horizon-ctl {name} timed out", "kind": "timeout"}
        except (FileNotFoundError, OSError) as error:
            return {"error": f"cannot run horizon-ctl: {error}", "kind": "missing"}
        reply = last_json(result.stdout)
        if reply is None:
            detail = (result.stderr or result.stdout or f"exit {result.returncode}").strip()
            return {"error": detail[-2000:], "kind": "protocol", "returncode": result.returncode}
        return reply

    def wait_for(self, condition: str, value: str | None, timeout: float) -> dict:
        args = [condition] + ([value] if value is not None else []) + ["--timeout", str(timeout)]
        return self.command("wait-for", *args, timeout=timeout + 5.0)


class GameE2ERunner:
    def __init__(self, game: Path, ctl: Path, resources: Path, world: str, seed: int,
                 timeout: float, home_root: Path | None, reuse_home: bool, keep_home: bool,
                 out_dir: Path, offscreen: bool, render_distance: int, *, stub: bool = False,
                 process_factory: Callable[..., object] = LocalProcess,
                 ctl_factory: Callable[..., object] = HorizonCtl,
                 launch_api_fn: Callable[..., dict] = default_launch_api,
                 sleep: Callable[[float], None] = time.sleep,
                 monotonic: Callable[[], float] = time.monotonic):
        self.game, self.ctl, self.resources = game, ctl, resources
        self.world, self.seed, self.timeout = world, seed, timeout
        self.wait_timeout = max(30.0, timeout)
        self.home_root, self.reuse_home, self.keep_home = home_root, reuse_home, keep_home
        self.out_dir, self.offscreen, self.render_distance = out_dir, offscreen, render_distance
        self.stub = stub
        self.process_factory, self.ctl_factory = process_factory, ctl_factory
        self.launch_api_fn, self.sleep, self.monotonic = launch_api_fn, sleep, monotonic
        self.home: TestHome | None = None
        self.process = None
        self.ctl_client = None
        self.processes: list[dict] = []
        self.backends: list[dict] = []
        self.traces: list[dict] = []
        self.screenshots: dict[str, str] = {}
        self.notes: list[str] = []
        self.phase = 0
        self.current_trace: Path | None = None
        self.current_stdout: Path | None = None
        self.current_stderr: Path | None = None
        self.hello_path: Path | None = None
        self.status_path: Path | None = None
        self.state_path: Path | None = None
        self.screen_path: Path | None = None
        self.worlds_path: Path | None = None
        self.open_path: Path | None = None
        self.trace_marks: dict[str, int] = {}
        self.save_position: list[int] | None = None
        self.started_at = monotonic()
        self._finished = self._archived = self._cleaned = False
        self.report: dict = {
            "schema": SCHEMA, "task": TASK_ID, "status": "blocked", "reason": "not started",
            "started_at": None, "finished_at": None, "duration_seconds": None,
            "candidate": {}, "isolation": {}, "processes": [], "backends": [], "traces": [],
            "checkpoints": {name: checkpoint(name) for name in CHECKPOINTS},
            "backend": {}, "trace": {}, "screenshots": {}, "stub": stub,
            "evidence_kind": "stub-control" if stub else "game-process", "notes": [],
        }

    def prepare(self) -> None:
        self.game, self.ctl, self.resources = self.game.resolve(), self.ctl.resolve(), self.resources.resolve()
        self.started_at = self.monotonic()
        self.report["started_at"] = datetime.now(timezone.utc).isoformat()
        if self.reuse_home and self.home_root is None:
            raise RunnerError("--reuse-home requires --home-root", blocked=True)
        if self.home_root is not None and not self.reuse_home:
            raise RunnerError("--home-root requires --reuse-home", blocked=True)
        root = Path(tempfile.mkdtemp(prefix="horizon-legacy-e2e-")) if self.home_root is None else self.home_root
        if self.home_root is not None:
            assert_scratch_root(root)
        self.home = TestHome.create(root, reused=self.reuse_home)
        self.report["isolation"] = {
            "root": str(self.home.root), "game_dir": str(self.home.game_dir),
            "home": str(self.home.home), "launcher_home": str(self.home.launcher_home),
            "xdg_data_home": str(self.home.data_home), "xdg_config_home": str(self.home.config_home),
            "runtime_dir": str(self.home.runtime_dir), "reused": self.home.reused,
        }
        if self.stub:
            self.report["candidate"] = {"game": str(self.game), "stub": True}
            return
        if not self.game.is_file():
            raise RunnerError(f"game binary not found: {self.game}", blocked=True)
        if not self.ctl.is_file():
            raise RunnerError(f"horizon-ctl not found: {self.ctl}", blocked=True)
        if not (self.resources / "horizon-resources.json").is_file():
            raise RunnerError(f"prepared resources missing: {self.resources}", blocked=True)
        trace = self.home.artifacts / "launch-api-trace.jsonl"
        socket = self.home.runtime_dir / f"launch-api-{os.getpid()}.sock"
        try:
            launch_api = self.launch_api_fn(self.game, self.home.env(trace, socket), self.home.game_dir)
        except (RuntimeError, subprocess.TimeoutExpired, OSError) as error:
            raise RunnerError(f"not a runnable Horizon game binary: {error}", blocked=True) from error
        self.report["candidate"] = {
            "game": str(self.game), "game_sha256": sha256(self.game),
            "ctl": str(self.ctl), "ctl_sha256": sha256(self.ctl),
            "resources": str(self.resources), "launch_api": launch_api,
        }

    def start_process(self) -> None:
        if self.home is None:
            raise RunnerError("test home was not prepared")
        self.phase += 1
        socket = self.home.runtime_dir / f"horizon-control-{self.phase}-{os.getpid()}.sock"
        trace = self.home.artifacts / f"startup-trace-{self.phase}.jsonl"
        stdout = self.home.artifacts / f"game-{self.phase}.stdout.log"
        stderr = self.home.artifacts / f"game-{self.phase}.stderr.log"
        env = self.home.env(trace, socket)
        command = [str(self.game), "--resources", str(self.resources),
                   "--control-socket", str(socket), "--home", str(self.home.home),
                   "--game-dir", str(self.home.game_dir),
                   "--render-distance", str(self.render_distance)]
        if self.offscreen:
            command.append("--offscreen")
        self.process = self.process_factory(command, self.home.game_dir, env, stdout, stderr)
        info = {"phase": self.phase, "pid": self.process.pid, "pgid": getattr(self.process, "pgid", None),
                "command": command, "socket": str(socket), "trace": str(trace),
                "stdout": str(stdout), "stderr": str(stderr), "returncode": None,
                "timed_out": False, "stopped": False}
        self.processes.append(info)
        self.report["processes"] = self.processes
        self.ctl_client = self.ctl_factory(self.ctl, socket, env, self.home.game_dir)
        self.current_trace, self.current_stdout, self.current_stderr = trace, stdout, stderr
        self.hello_path = self.home.artifacts / f"hello-{self.phase}.json"
        self.status_path = self.home.artifacts / f"status-{self.phase}.json"
        self.state_path = self.home.artifacts / f"state-{self.phase}.json"
        self.screen_path = self.home.artifacts / f"screen-{self.phase}.json"
        self.worlds_path = self.home.artifacts / f"worlds-{self.phase}.json"
        self.open_path = self.home.artifacts / f"open-{self.phase}.json"

    def stop_process(self) -> None:
        if self.process is None:
            return
        process, self.process = self.process, None
        process.stop()
        info = self.processes[-1] if self.processes else {}
        info["returncode"] = process.poll()
        info["stopped"] = True
        self.report["processes"] = self.processes

    def command(self, name: str, *args: str, timeout: float | None = None) -> dict:
        if self.ctl_client is None:
            raise RunnerError("control client was not created")
        reply = self.ctl_client.command(name, *args, timeout=timeout)
        if not isinstance(reply, dict):
            raise RunnerError(f"{name}: reply is not a JSON object")
        if reply.get("error"):
            raise RunnerError(f"{name}: {reply['error']}")
        return reply

    def wait_control(self) -> None:
        deadline = self.monotonic() + self.timeout
        while self.monotonic() < deadline:
            if self.process is not None and self.process.poll() is not None:
                raise RunnerError(f"game exited before the control socket opened (code {self.process.poll()})")
            try:
                hello = self.command("hello", timeout=2.0)
            except RunnerError:
                hello = None
            if isinstance(hello, dict) and hello.get("protocol"):
                if self.hello_path:
                    write_json(self.hello_path, hello)
                return
            self.sleep(0.2)
        if self.processes:
            self.processes[-1]["timed_out"] = True
        self.stop_process()
        raise RunnerError("control socket did not open within the startup timeout")

    def refresh_trace(self) -> None:
        if self.current_trace is None:
            return
        marks = read_trace_marks(self.current_trace)
        self.trace_marks = marks
        entry = {"phase": self.phase, "path": str(self.current_trace), "marks": marks}
        self.traces = [item for item in self.traces if item.get("phase") != self.phase] + [entry]
        self.report["trace"], self.report["traces"] = entry, self.traces

    def log_text(self) -> str:
        if self.home is None:
            return ""
        parts = []
        for path in (self.home.home / "logs" / "latest.log", self.current_stderr, self.current_stdout):
            if path is not None:
                text = tail_text(path)
                if text:
                    parts.append(text)
        return "\n".join(parts)

    def screenshot(self, name: str) -> None:
        if self.home is None:
            return
        path = self.home.artifacts / f"{name}.png"
        try:
            self.command("screenshot", str(path))
        except RunnerError as error:
            self.notes.append(f"screenshot {name} failed: {error}")
            return
        self.screenshots[name] = str(path)
        self.report["screenshots"] = self.screenshots

    def check_startup(self) -> None:
        hello = self.command("hello")
        status = self.command("status")
        if self.hello_path:
            write_json(self.hello_path, hello)
        if self.status_path:
            write_json(self.status_path, status)
        self.refresh_trace()
        backend = inspect_backend(status, self.log_text())
        self.backends.append(backend)
        self.report["backend"], self.report["backends"] = backend, self.backends
        if backend["state"] == "fallback":
            raise RunnerError(f"erroneous fallback: {backend['reason']}")
        if not backend["confirmed"]:
            raise RunnerError(f"legacy backend not confirmed: {backend['reason']}", blocked=True)
        self.report["checkpoints"]["startup"] = checkpoint("startup", True, {
            "hello": hello,
            "status": {"screen": status.get("screen"), "renderer": status.get("renderer"),
                       "display": status.get("display")},
            "trace": {name: self.trace_marks.get(name) for name in ("loading_screen.shown", "title.shown")},
        })

    def check_menu(self) -> None:
        screen = self.command("screen")
        if self.screen_path:
            write_json(self.screen_path, screen)
        title = screen.get("screen") == "title"
        if not title and self.ctl_client is not None:
            reply = self.ctl_client.wait_for("screen", "title", self.wait_timeout)
            if isinstance(reply, dict) and not reply.get("error"):
                title, screen = True, reply
        self.refresh_trace()
        if not title:
            raise RunnerError("menu checkpoint missing: title screen not observed")
        self.report["checkpoints"]["menu"] = checkpoint("menu", True, {
            "screen": screen, "trace": self.trace_marks.get("title.shown")})
        self.screenshot("menu")

    def check_world(self) -> None:
        open_reply = self.command("open", self.world, str(self.seed))
        if self.open_path:
            write_json(self.open_path, open_reply)
        if self.ctl_client is None:
            raise RunnerError("control client was not created")
        waited = self.ctl_client.wait_for("loaded", None, self.wait_timeout)
        if not isinstance(waited, dict) or waited.get("error"):
            raise RunnerError(f"world checkpoint missing: {waited.get('error') if isinstance(waited, dict) else waited}")
        state, status = self.command("state"), self.command("status")
        if self.state_path:
            write_json(self.state_path, state)
        if self.status_path:
            write_json(self.status_path, status)
        self.refresh_trace()
        player = state.get("player")
        if not player:
            raise RunnerError("world checkpoint missing: no player state")
        if (status.get("chunks") or 0) <= 0 and (state.get("chunks") or 0) <= 0:
            raise RunnerError("world checkpoint missing: no chunks loaded")
        self.report["checkpoints"]["world"] = checkpoint("world", True, {
            "open": open_reply, "waited": waited,
            "state": {"player": player, "dimension": state.get("dimension"), "gamemode": state.get("gamemode")},
            "status": {"chunks": status.get("chunks"), "pending_chunks": status.get("pending_chunks"),
                       "screen": status.get("screen")},
            "trace": {name: self.trace_marks.get(name) for name in ("world.first_frame", "world.view_full")},
        })
        self.screenshot("world")

    def save_checkpoint(self) -> None:
        state = self.command("state")
        player = state.get("player") or {}
        position = player.get("block_pos")
        if not isinstance(position, (list, tuple)) or len(position) != 3:
            raise RunnerError("save checkpoint missing: no player block position")
        x, y, z = (int(value) for value in position)
        self.save_position = [x, y, z]
        self.command("setblock", str(x), str(y), str(z), "minecraft:stone")
        self.command("save-all")
        worlds = self.command("worlds")
        if self.worlds_path:
            write_json(self.worlds_path, worlds)
        folders = [item.get("folder") for item in worlds.get("worlds", []) if isinstance(item, dict)]
        if self.world not in folders:
            raise RunnerError(f"save checkpoint missing: world {self.world} is not in worlds")
        if self.home is None:
            raise RunnerError("test home was not prepared")
        level = self.home.game_dir / "saves" / self.world / "level.dat"
        if not level.is_file():
            raise RunnerError(f"save checkpoint missing: {level} was not written")
        level_info = {"path": str(level), "bytes": level.stat().st_size, "mtime": level.stat().st_mtime}
        self.command("quit")
        returncode = self.wait_exit()
        if returncode is None:
            raise RunnerError("save checkpoint missing: game did not exit after quit")
        if returncode != 0:
            raise RunnerError(f"save checkpoint failed: game exited with code {returncode}")
        self.report["checkpoints"]["save"] = checkpoint("save", True, {
            "position": [x, y, z], "setblock": "minecraft:stone", "save": "save-all",
            "worlds": worlds, "level_dat": level_info, "returncode": returncode})

    def wait_exit(self) -> int | None:
        if self.process is None:
            return None
        result = self.process.wait(self.wait_timeout)
        if result is None and self.processes:
            self.processes[-1]["timed_out"] = True
        return result

    def reopen_and_verify(self) -> None:
        if self.save_position is None:
            raise RunnerError("save checkpoint has no position to reopen")
        self.start_process()
        self.wait_control()
        self.check_startup()
        self.check_menu()
        self.check_world()
        x, y, z = self.save_position
        block = self.command("block", str(x), str(y), str(z))
        if block.get("block") != "minecraft:stone":
            raise RunnerError(f"save reopen failed: block is {block.get('block')!r}, expected minecraft:stone")
        self.command("quit")
        returncode = self.wait_exit()
        if returncode is None:
            raise RunnerError("save reopen failed: game did not exit after quit")
        if returncode != 0:
            raise RunnerError(f"save reopen failed: game exited with code {returncode}")
        self.report["checkpoints"]["save"]["evidence"]["reopen"] = {"block": block, "returncode": returncode}

    def run(self) -> dict:
        status, reason = "failed", "runner stopped before a verdict"
        try:
            self.prepare()
            self.start_process()
            self.wait_control()
            self.check_startup()
            self.check_menu()
            self.check_world()
            self.save_checkpoint()
            self.stop_process()
            self.reopen_and_verify()
            status, reason = "passed", "all checkpoints observed"
        except RunnerError as error:
            status, reason = ("blocked" if error.blocked else "failed"), str(error)
        except Exception as error:  # noqa: BLE001 - report, never turn a crash into success
            status, reason = "failed", f"runner error: {error}"
        finally:
            self.stop_process()
            self.finish(status, reason)
            self.archive()
            self.cleanup()
        return self.report

    def finish(self, status: str, reason: str) -> dict:
        if self._finished:
            return self.report
        self._finished = True
        self.report.update(status=status, reason=reason,
                           finished_at=datetime.now(timezone.utc).isoformat(),
                           duration_seconds=round(self.monotonic() - self.started_at, 3),
                           processes=self.processes, backends=self.backends, traces=self.traces,
                           screenshots=self.screenshots)
        self.report["notes"] = list(self.notes)
        if self.stub:
            self.report["notes"].append("stub control: this report is a unit-test fixture, not Game evidence")
        if self.offscreen:
            self.report["notes"].append("offscreen run: Game E2E, but not a visible-window B1 qualification")
        self.report["notes"] += ["Win7/X1650 qualification is not attempted by this runner",
                                 "legacy backend confirmation is a requested integrator hook"]
        self.out_dir.mkdir(parents=True, exist_ok=True)
        write_json(self.out_dir / "report.json", self.report)
        observed = ", ".join(name for name in CHECKPOINTS if self.report["checkpoints"][name]["observed"])
        print(f"legacy Game E2E ({TASK_ID}): {status}")
        print(f"reason: {reason}")
        print(f"home: {self.report.get('isolation', {}).get('root', '')}")
        print(f"report: {self.out_dir / 'report.json'}")
        print(f"checkpoints: {observed or 'none'}")
        print(f"backend: {self.report.get('backend', {}).get('state', 'unknown')}")
        print(f"stub: {self.stub}")
        return self.report

    def archive(self) -> None:
        if self._archived or self.home is None:
            return
        self._archived = True
        if self.out_dir.resolve() == self.home.root or self.home.root in self.out_dir.resolve().parents:
            return
        self.out_dir.mkdir(parents=True, exist_ok=True)
        for path in self.home.artifacts.iterdir():
            if path.is_file():
                shutil.copy2(path, self.out_dir / path.name)
        latest = self.home.home / "logs" / "latest.log"
        if latest.is_file():
            shutil.copy2(latest, self.out_dir / "game-latest.log")
        crash = newest_crash_report(self.home.home)
        if crash is not None:
            shutil.copy2(crash, self.out_dir / crash.name)

    def cleanup(self) -> None:
        if self._cleaned or self.home is None:
            return
        self._cleaned = True
        if self.home.reused or self.keep_home:
            return
        if self.out_dir.resolve() == self.home.root or self.home.root in self.out_dir.resolve().parents:
            return
        if self.report["status"] == "passed":
            shutil.rmtree(self.home.root, ignore_errors=True)


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        self.print_help(sys.stderr)
        raise SystemExit(3)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = _Parser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--game", type=Path, required=True, help="horizon-client binary")
    parser.add_argument("--ctl", type=Path, help="horizon-ctl binary (default: beside the game)")
    parser.add_argument("--resources", type=Path, required=True, help="prepared resources directory")
    parser.add_argument("--world", default="T01a", help="test world folder under saves/")
    parser.add_argument("--seed", type=int, default=1234, help="seed for a new test world")
    parser.add_argument("--render-distance", type=int, default=6, help="chunks (2..128)")
    parser.add_argument("--timeout", type=float, default=180.0, help="startup and command timeout")
    parser.add_argument("--home-root", type=Path, help="reuse this scratch root (with --reuse-home)")
    parser.add_argument("--reuse-home", action="store_true", help="reuse --home-root and its test save")
    parser.add_argument("--keep-home", action="store_true", help="keep the scratch home after the run")
    parser.add_argument("--out", type=Path, default=ROOT / "target/legacy-game-e2e")
    parser.add_argument("--offscreen", action="store_true", help="render without a visible window")
    args = parser.parse_args(argv)
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if not 2 <= args.render_distance <= 128:
        parser.error("--render-distance must be 2..128")
    if not args.world:
        parser.error("--world must not be empty")
    if args.home_root is not None and not args.reuse_home:
        parser.error("--home-root requires --reuse-home")
    if args.reuse_home and args.home_root is None:
        parser.error("--reuse-home requires --home-root")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    runner = GameE2ERunner(args.game, find_ctl(args.game, args.ctl), args.resources, args.world,
                           args.seed, args.timeout, args.home_root, args.reuse_home, args.keep_home,
                           args.out, args.offscreen, args.render_distance)
    try:
        report = runner.run()
    except KeyboardInterrupt:
        runner.stop_process()
        report = runner.finish("failed", "interrupted")
    return 0 if report["status"] == "passed" else (2 if report["status"] == "blocked" else 1)


if __name__ == "__main__":
    raise SystemExit(main())
