"""Measure real-engine dashboard reads with fresh homes and the real feature runtime.

python3 engine/scripts/qa_native_ui.py target/release/factr BUNDLED_PYTHON_DIR OUT.json
Add --before to measure the forwarded delivery-targets route before its native port.
No user configuration, credentials, external providers or benchmark files are used.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import threading
import time
import urllib.request


def descendants(pid):
    rows = []
    for line in subprocess.check_output(
        ["/bin/ps", "-axo", "pid=,ppid=,rss=,comm="], text=True
    ).splitlines():
        fields = line.strip().split(None, 3)
        if len(fields) == 4:
            rows.append((int(fields[0]), int(fields[1]), int(fields[2]), fields[3]))
    selected = {pid}
    while True:
        children = {p for p, parent, _, _ in rows if parent in selected}
        if children <= selected:
            return [r for r in rows if r[0] in selected]
        selected |= children


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", type=Path)
    parser.add_argument("python_bundle", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--before", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    engine, bundle, output = args.engine.resolve(), args.python_bundle.resolve(), args.output.resolve()
    root = Path(tempfile.mkdtemp(prefix="factri-native-ui-", dir="/private/tmp"))
    for name in ("home", "factr", "config", "userdata"):
        (root / name).mkdir()
    pristine = json.loads((repo / "engine/crates/factr-gateway/tests/fixtures/ui_routes/pristine.json").read_text())
    defaults = root / "defaults.json"
    defaults.write_text(json.dumps(pristine["_defaults"]))
    env = {"PATH": os.environ["PATH"], "HOME": str(root / "home"),
           "FACTR_HOME": str(root / "factr"), "FACTR_CONFIG_HOME": str(root / "config"),
           "FACTR_DESKTOP_USER_DATA_DIR": str(root / "userdata"),
           "FACTR_BACKEND_PYTHON": str(bundle / "runtime/bin/python3.12"),
           "FACTR_BACKEND_DEFAULTS": str(defaults), "FACTR_PROVIDER": "qa-no-provider",
           "FACTR_TRACE_FORWARD": "1", "PYTHONDONTWRITEBYTECODE": "1",
           "PYTHONPATH": f"{repo / 'backend'}:{bundle / 'packages'}"}
    token = "native-ui-isolated-test-token-0000000000000000"
    samples, stop, python_commands = [], threading.Event(), {}
    request_phase = threading.Event()
    log_path = root / "engine.log"
    started = time.perf_counter()
    with log_path.open("w") as log:
        proc = subprocess.Popen([str(engine), "serve", "--port", "0", "--token-stdin"],
                                stdin=subprocess.PIPE, stdout=log, stderr=log,
                                cwd=root, env=env, text=True, start_new_session=True)
        proc.stdin.write(token + "\n")
        proc.stdin.close()
        def sample():
            while not stop.is_set():
                rows = descendants(proc.pid)
                samples.append((request_phase.is_set(), rows))
                for pid, _, _, command in rows:
                    if "python" in command.lower() and pid not in python_commands:
                        args_out = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "command="],
                                                  capture_output=True, text=True)
                        python_commands[pid] = args_out.stdout.strip()
                stop.wait(0.1)
        sampler = threading.Thread(target=sample)
        sampler.start()
        result = {"root": str(root), "engine": str(engine), "routes": {}}
        try:
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                ready = re.search(r"FACTR_BACKEND_READY port=(\d+)", log_path.read_text())
                if ready:
                    break
                assert proc.poll() is None, f"engine exited; see {log_path}"
                time.sleep(0.05)
            assert ready, f"engine readiness timed out; see {log_path}"
            result["ready_ms"] = round((time.perf_counter() - started) * 1000, 2)
            # Tool-environment setup may briefly probe Python at boot. Let it finish;
            # every Python child observed during the dashboard requests is a failure.
            quiet_since = None
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if any("python" in r[3].lower() for r in descendants(proc.pid)):
                    quiet_since = None
                elif quiet_since is None:
                    quiet_since = time.monotonic()
                elif time.monotonic() - quiet_since >= 0.5:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError(f"startup Python never settled; see {log_path}")
            request_phase.set()
            routes = ["/api/cron/delivery-targets"]
            if not args.before:
                routes += ["/api/config/schema", "/api/model/auxiliary", "/api/model/moa",
                           "/api/audio/elevenlabs/voices", "/api/pairing", "/api/cron/jobs", "/api/cron/blueprints"]
            for route in routes:
                timings = []
                for _ in range(5):
                    before = time.perf_counter()
                    request = urllib.request.Request(f"http://127.0.0.1:{ready[1]}{route}?feature=1",
                                                     headers={"Authorization": "Bearer " + token})
                    with urllib.request.urlopen(request, timeout=120) as response:
                        assert response.status == 200
                        body = json.load(response)
                    timings.append(round((time.perf_counter() - before) * 1000, 2))
                result["routes"][route] = {"ms": timings}
                if route == "/api/cron/delivery-targets":
                    assert body == pristine["cron_delivery_targets"]["body"], route
                else:
                    key = {"/api/config/schema": "schema"}.get(route, route.removeprefix("/api/").replace("/", "_"))
                    if key == "audio_elevenlabs_voices":
                        key = "elevenlabs_voices"
                    assert body == pristine[key]["body"], route
            time.sleep(1)
            python = [row for phase, sample in samples if phase for row in sample if "python" in row[3].lower()]
            startup_python = [row for phase, sample in samples if not phase for row in sample if "python" in row[3].lower()]
            forwards = [line for line in log_path.read_text().splitlines() if "forward HTTP" in line]
            result.update(python_pids=sorted({r[0] for r in python}),
                          peak_python_rss_mb=round(max((r[2] for r in python), default=0) / 1024, 2),
                          peak_engine_rss_mb=round(max((r[2] for _, sample in samples for r in sample if r[0] == proc.pid), default=0) / 1024, 2),
                          forwards=forwards, samples=len(samples))
            result["startup_python_pids"] = sorted({r[0] for r in startup_python})
            result["startup_interpreter_probes"] = sum("python.get_interpreter_info" in c for c in python_commands.values())
            assert bool(python) == args.before, result
            assert bool(forwards) == args.before, result
        finally:
            stop.set()
            sampler.join()
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
            seen = {r[0] for _, sample in samples for r in sample}
            remaining = []
            for pid in seen:
                try:
                    os.kill(pid, 0)
                    remaining.append(pid)
                except ProcessLookupError:
                    pass
            result["remaining_processes"] = len(remaining)
            assert result["remaining_processes"] == 0
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
