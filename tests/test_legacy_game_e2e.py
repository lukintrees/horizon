"""T01a runner: isolation, timeout, crash, checkpoint and backend evidence."""
import importlib.util
import os
from pathlib import Path
import signal
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
SPEC = importlib.util.spec_from_file_location("legacy_game_e2e", ROOT / "tools/legacy_game_e2e.py")
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


def status_legacy():
    return {
        "screen": "title", "chunks": 10, "pending_chunks": 0,
        "renderer": {"backend": "LegacyGl", "gpu": "test", "vendor": "test", "driver": "test"},
        "legacy": {"confirmed": True, "backend": "legacy-gl", "gl_version": "2.1",
                   "glsl_version": "1.20", "vendor": "test", "renderer": "test"},
        "display": {"width": 800, "height": 600},
    }


def status_unconfirmed():
    return {"screen": "title", "chunks": 10, "renderer": {"backend": "Gl"},
            "display": {"width": 800, "height": 600}}


def status_fallback():
    return {"screen": "title", "chunks": 10, "renderer": {"backend": "Vulkan"},
            "display": {"width": 800, "height": 600}}


class FakeProcess:
    def __init__(self, command, cwd, env, stdout_path, stderr_path, exit_code=0, exit_after=None):
        self.command, self.cwd, self.env = command, cwd, env
        self.stdout_path, self.stderr_path = Path(stdout_path), Path(stderr_path)
        self.stdout_path.write_text("")
        self.stderr_path.write_text("")
        self.pid, self.pgid = 4242, 4242
        self.exit_code, self.exit_after = exit_code, exit_after
        self.polls = 0
        self.stopped = False
        self.exited = False

    def poll(self):
        self.polls += 1
        if self.stopped or self.exited:
            return self.exit_code
        if self.exit_after is not None and self.polls > self.exit_after:
            self.exited = True
            return self.exit_code
        return None

    def wait(self, timeout):
        self.exited = True
        return self.exit_code

    def stop(self):
        self.stopped = True
        if self.exit_code is None:
            self.exit_code = -signal.SIGTERM


class FakeCtl:
    def __init__(self, ctl, socket, env, cwd, phase, state, loaded_ok=True, hello_ok=True):
        self.phase, self.state, self.loaded_ok, self.hello_ok = phase, state, loaded_ok, hello_ok
        self.cwd = cwd

    def command(self, name, *args, timeout=None):
        if name == "hello":
            if not self.hello_ok:
                return {"error": "not yet", "kind": "connection"}
            return {"protocol": "0.1.0", "release": "test", "features": []}
        if name == "status":
            return self.state["status"]
        if name == "screen":
            return {"ok": True, "screen": "title"}
        if name == "open":
            return {"ok": True}
        if name == "state":
            return {"ok": True, "yaw": 0.0, "dimension": "minecraft:overworld",
                    "gamemode": "survival",
                    "player": {"block_pos": [10, 64, 10], "position": [10.5, 64, 10.5]}}
        if name == "setblock":
            return {"ok": True}
        if name in ("save", "save-all"):
            level = self.cwd / "saves" / "T01a" / "level.dat"
            level.parent.mkdir(parents=True, exist_ok=True)
            level.write_text("stub")
            return {"ok": True}
        if name == "worlds":
            return {"ok": True, "worlds": [{"folder": "T01a"}]}
        if name == "block":
            return {"ok": True, "block": "minecraft:stone"}
        if name == "quit":
            return {"ok": True}
        if name == "screenshot":
            path = Path(args[-1])
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"\x89PNG\r\n\x1a\n")
            return {"ok": True, "path": str(path)}
        raise AssertionError(f"unexpected command {name} {args}")

    def wait_for(self, condition, value, timeout):
        if condition == "loaded" and not self.loaded_ok:
            return {"error": "not ready", "kind": "timeout"}
        return {"ok": True}


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class RunnerTests(unittest.TestCase):
    def make_runner(self, *, status=None, exit_code=0, exit_after=None, timeout=5.0,
                    loaded_ok=True, hello_ok=True):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        state = {"status": status if status is not None else status_legacy()}
        processes = []

        def process_factory(command, cwd, env, stdout_path, stderr_path):
            process = FakeProcess(command, cwd, env, stdout_path, stderr_path,
                                  exit_code=exit_code, exit_after=exit_after)
            processes.append(process)
            return process

        def ctl_factory(ctl, socket, env, cwd):
            return FakeCtl(ctl, socket, env, cwd, len(processes), state, loaded_ok, hello_ok)

        clock = FakeClock()
        test_runner = runner.GameE2ERunner(
            Path("/stub/horizon-client"), Path("/stub/horizon-ctl"), Path("/stub/resources"),
            "T01a", 1234, timeout, None, False, True, root / "out", False, 6,
            stub=True, process_factory=process_factory, ctl_factory=ctl_factory,
            sleep=clock.sleep, monotonic=clock.monotonic)
        return test_runner, processes

    def wait_until(self, predicate, timeout=10.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.05)
        self.fail("condition was not met")

    def test_stub_control_reaches_all_checkpoints(self):
        test_runner, processes = self.make_runner()
        report = test_runner.run()
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["evidence_kind"], "stub-control")
        self.assertTrue(report["stub"])
        self.assertTrue(report["backend"]["confirmed"])
        for name in runner.CHECKPOINTS:
            self.assertTrue(report["checkpoints"][name]["observed"], name)
        self.assertIn("reopen", report["checkpoints"]["save"]["evidence"])
        self.assertEqual(len(processes), 2)
        self.assertTrue(all(process.stopped for process in processes))

    def test_timeout_fails_and_stops_its_process(self):
        test_runner, processes = self.make_runner(exit_code=None, timeout=5.0, hello_ok=False)
        report = test_runner.run()
        self.assertEqual(report["status"], "failed")
        self.assertIn("control socket", report["reason"])
        self.assertTrue(processes[0].stopped)
        self.assertTrue(report["processes"][0]["timed_out"])

    def test_crash_fails_and_reports_returncode(self):
        test_runner, processes = self.make_runner(exit_code=1, exit_after=0)
        report = test_runner.run()
        self.assertEqual(report["status"], "failed")
        self.assertIn("exited before the control socket", report["reason"])
        self.assertEqual(report["processes"][0]["returncode"], 1)
        self.assertTrue(processes[0].stopped)

    def test_missing_world_checkpoint_fails(self):
        test_runner, processes = self.make_runner(loaded_ok=False)
        report = test_runner.run()
        self.assertEqual(report["status"], "failed")
        self.assertIn("world checkpoint missing", report["reason"])
        self.assertIn("save", report["checkpoints"])
        self.assertTrue(processes[0].stopped)

    def test_unconfirmed_legacy_backend_is_blocked(self):
        test_runner, processes = self.make_runner(status=status_unconfirmed())
        report = test_runner.run()
        self.assertEqual(report["status"], "blocked")
        self.assertIn("legacy backend not confirmed", report["reason"])
        self.assertEqual(report["backend"]["state"], "unconfirmed")
        self.assertTrue(processes[0].stopped)

    def test_modern_backend_fallback_fails(self):
        test_runner, processes = self.make_runner(status=status_fallback())
        report = test_runner.run()
        self.assertEqual(report["status"], "failed")
        self.assertIn("erroneous fallback", report["reason"])
        self.assertEqual(report["backend"]["state"], "fallback")
        self.assertTrue(processes[0].stopped)

    def test_inspect_backend_separates_unconfirmed_from_fallback(self):
        cases = [
            (status_unconfirmed(), "unconfirmed", False),
            (status_fallback(), "fallback", False),
            (status_legacy(), "confirmed", True),
            ({"legacy": {"confirmed": True, "backend": "legacy-gl"}}, "incomplete", False),
            ({"legacy": {"confirmed": True, "backend": "legacy-gl", "gl_version": "2.1",
                         "glsl_version": "1.20", "vendor": "v", "renderer": "r"},
              "renderer": {"backend": "Vulkan"}}, "fallback", False),
        ]
        for status, state, confirmed in cases:
            with self.subTest(state=state):
                result = runner.inspect_backend(status)
                self.assertEqual(result["state"], state)
                self.assertEqual(result["confirmed"], confirmed)

    def test_read_trace_marks(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "trace.jsonl"
            path.write_text('{"ph":"i","name":"title.shown","ts":100}\n'
                            '{"ph":"i","name":"world.first_frame","ts":200}\n')
            marks = runner.read_trace_marks(path)
        self.assertEqual(marks["title.shown"], 100)
        self.assertEqual(marks["world.first_frame"], 200)

    def test_test_home_env_is_isolated(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            home = runner.TestHome.create(root, reused=False)
            trace = home.artifacts / "trace.jsonl"
            socket = home.runtime_dir / "ctl.sock"
            env = home.env(trace, socket)
            for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
                self.assertTrue(Path(env[key]).is_relative_to(root), key)
            self.assertEqual(env["HORIZON_TELEMETRY_URL"], "off")
            self.assertEqual(env["HORIZON_NO_AUDIO"], "1")
            self.assertEqual(env["HORIZON_LAUNCHER_NO_SELF_UPDATE"], "1")
            self.assertEqual(env["HORIZON_STARTUP_TRACE"], str(trace))
            self.assertEqual(env["HORIZON_CONTROL"], str(socket))

    def test_fresh_home_root_must_be_empty(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "old").write_text("x")
            with self.assertRaises(runner.RunnerError):
                runner.TestHome.create(root, reused=False)
            runner.TestHome.create(root, reused=True)

    def test_assert_scratch_root_rejects_real_home(self):
        with self.assertRaises(runner.RunnerError):
            runner.assert_scratch_root(Path.home())
        with tempfile.TemporaryDirectory() as temp:
            runner.assert_scratch_root(Path(temp))

    def test_local_process_stop_terminates_only_its_own_process(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            process = runner.LocalProcess(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                cwd=root, env=dict(os.environ),
                stdout_path=root / "out.log", stderr_path=root / "err.log")
            try:
                if os.name != "nt":
                    self.assertEqual(os.getpgid(process.pid), process.pid)
                self.assertIsNone(process.poll())
                process.stop()
                self.wait_until(lambda: process.poll() is not None)
            finally:
                # Windows cannot remove open log files. Stop and close them
                # before TemporaryDirectory attempts to remove the scratch root.
                process.stop()


if __name__ == "__main__":
    unittest.main()
