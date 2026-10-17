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

ROUTES = {
    "schema": "/api/config/schema",
    "model_auxiliary": "/api/model/auxiliary",
    "model_moa": "/api/model/moa",
    "elevenlabs_voices": "/api/audio/elevenlabs/voices",
    "pairing": "/api/pairing",
    "cron_jobs": "/api/cron/jobs",
    "cron_blueprints": "/api/cron/blueprints",
}

FIXTURES = {
    "pristine": {},
    "configured": {
        "config.yaml": "model:\n  default: gpt-5.5\n  provider: openai-codex\nauxiliary:\n  vision:\n    provider: openrouter\n    model: google/gemini-3-flash\n"
                       "moa:\n  default_preset: default\n  presets:\n    default:\n      reference_models:\n        - {provider: openrouter, model: deepseek/deepseek-v4-pro}\n"
                       "      aggregator: {provider: openrouter, model: anthropic/claude-opus-4.8}\ndisplay:\n  language: en\n",
    },
}


def run_one(home: str, out: str) -> None:
    from starlette.testclient import TestClient
    from factr_backend.web_server import app, _SESSION_HEADER_NAME, _SESSION_TOKEN

    client = TestClient(app)
    client.headers[_SESSION_HEADER_NAME] = _SESSION_TOKEN
    from factr_backend.config import DEFAULT_CONFIG

    result = {"_defaults": DEFAULT_CONFIG}
    for name, path in ROUTES.items():
        r = client.get(path)
        result[name] = {"status": r.status_code, "body": r.json() if r.headers.get("content-type", "").startswith("application/json") else r.text}
    Path(out).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")


def main() -> int:
    out_dir = Path(sys.argv[1])
    if len(sys.argv) > 3 and sys.argv[2] == "--one":
        run_one(sys.argv[3], sys.argv[1])
        return 0
    drift = False
    for fixture, files in FIXTURES.items():
        home = Path(tempfile.mkdtemp(prefix=f"ui-routes-{fixture}-"))
        try:
            for rel, text in files.items():
                (home / rel).parent.mkdir(parents=True, exist_ok=True)
                (home / rel).write_text(text)
            env = {**os.environ, "FACTR_CONFIG_HOME": str(home / "c"), "FACTR_HOME": str(home / "e"), "HOME": str(home)}
            (home / "c").mkdir()
            if "config.yaml" in files:
                shutil.move(str(home / "config.yaml"), str(home / "c" / "config.yaml"))
            for k in ("ELEVENLABS_API_KEY", "FACTR_MANAGED_DIR"):
                env.pop(k, None)
            target = out_dir / f"{fixture}.json"
            tmp = Path(tempfile.mkstemp(suffix=".json")[1])
            subprocess.run([sys.executable, __file__, str(tmp), "--one", str(home)], check=True, env=env)
            if "--check" in sys.argv:
                drift |= not target.exists() or json.loads(target.read_text()) != json.loads(tmp.read_text())
            else:
                out_dir.mkdir(parents=True, exist_ok=True)
                shutil.move(str(tmp), target)
        finally:
            shutil.rmtree(home, ignore_errors=True)
    return 1 if drift else 0


if __name__ == "__main__":
    sys.exit(main())
