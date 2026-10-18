"""Dump the dashboard JSON of the read-only UI routes the Rust engine serves natively.

    python scripts/dump_ui_routes.py OUT_DIR            # (re)write the goldens
    python scripts/dump_ui_routes.py OUT_DIR --check    # exit 1 when they drifted

Each fixture is a throwaway FACTR_CONFIG_HOME; the real FastAPI app answers through Starlette's TestClient.
engine/crates/factr-gateway/tests/ui_routes.rs asserts the engine's own answers are equal to these files.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

ROUTES = {
    "schema": "/api/config/schema",
    "model_auxiliary": "/api/model/auxiliary",
    "model_moa": "/api/model/moa",
    "elevenlabs_voices": "/api/audio/elevenlabs/voices",
    "pairing": "/api/pairing",
    "cron_jobs": "/api/cron/jobs",
    "cron_blueprints": "/api/cron/blueprints",
}

FIXTURES = json.loads((Path(__file__).resolve().parents[2] / "engine/crates/factr-gateway/tests/fixtures/ui_routes/cases.json").read_text())


def run_one(fixture: str, out: str) -> None:
    from starlette.testclient import TestClient
    from factr_backend.web_server import app, _SESSION_HEADER_NAME, _SESSION_TOKEN

    client = TestClient(app)
    client.headers[_SESSION_HEADER_NAME] = _SESSION_TOKEN
    from factr_backend.config import DEFAULT_CONFIG

    case = FIXTURES[fixture]
    result = {"_defaults": DEFAULT_CONFIG} if fixture in ("pristine", "configured") else {}
    with patch("time.time", return_value=1800000060):
        for name in case.get("routes", ROUTES):
            path = ROUTES[name] + ("?" + case["query"] if case.get("query") else "")
            r = client.get(path)
            result[name] = {"status": r.status_code, "body": r.json() if r.headers.get("content-type", "").startswith("application/json") else r.text}
    # Cron rows include absolute homes. Keep temporary paths out of the contract.
    data = json.dumps(result, indent=1, sort_keys=True).replace(os.environ["FIXTURE_ROOT"], "<fixture>")
    Path(out).write_text(data + "\n")


def main() -> int:
    out_dir = Path(sys.argv[1])
    if len(sys.argv) > 3 and sys.argv[2] == "--one":
        run_one(sys.argv[3], sys.argv[1])
        return 0
    drift = False
    for fixture, case in FIXTURES.items():
        home = Path(tempfile.mkdtemp(prefix=f"ui-routes-{fixture}-", dir="/private/tmp"))
        try:
            root = home / "c"
            config_home = root / case.get("home", "")
            config_home.mkdir(parents=True)
            for rel, text in case.get("files", {}).items():
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_text(text)
            # Whitelist rather than inherit credentials or platform settings from the caller.
            env = {k: os.environ[k] for k in ("PATH", "PYTHONPATH") if k in os.environ}
            env.update(FACTR_CONFIG_HOME=str(config_home), FACTR_HOME=str(home / "e"), HOME=str(home),
                       PYTHONDONTWRITEBYTECODE="1", FIXTURE_ROOT=str(root))
            if case.get("managed"):
                env["FACTR_MANAGED_DIR"] = str(root / case["managed"])
            target = out_dir / f"{fixture}.json"
            tmp = home / "result.json"
            subprocess.run([sys.executable, __file__, str(tmp), "--one", fixture], check=True, env=env, cwd=home)
            if "--check" in sys.argv:
                changed = not target.exists() or json.loads(target.read_text()) != json.loads(tmp.read_text())
                drift |= changed
                print(f"{fixture}: {'DRIFT' if changed else 'OK'}")
            else:
                out_dir.mkdir(parents=True, exist_ok=True)
                shutil.move(str(tmp), target)
                print(f"{fixture}: wrote golden")
        finally:
            shutil.rmtree(home, ignore_errors=True)
    return 1 if drift else 0


if __name__ == "__main__":
    sys.exit(main())
