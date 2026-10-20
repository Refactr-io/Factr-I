"""The Rust engine answers some dashboard GETs from files (engine/crates/factr-gateway/src/ui_rest.rs) and
embeds goldens of these routes. This fails when the Python routes drift from them: regenerate with
`python scripts/dump_ui_routes.py engine/crates/factr-gateway/tests/fixtures/ui_routes` and rebuild
src/ui_golden/{schema,cron_blueprints,model_moa}.json from the pristine fixture."""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GATEWAY = ROOT / "engine/crates/factr-gateway"


def test_ui_route_goldens_match_the_python_routes():
    fixtures = GATEWAY / "tests/fixtures/ui_routes"
    r = subprocess.run([sys.executable, str(ROOT / "backend/scripts/dump_ui_routes.py"), str(fixtures), "--check"])
    assert r.returncode == 0, "ui route goldens drifted from the Python routes"
    pristine = json.loads((fixtures / "pristine.json").read_text())
    for key in ("schema", "cron_blueprints", "model_moa"):
        embedded = json.loads((GATEWAY / f"src/ui_golden/{key}.json").read_text())
        assert embedded == pristine[key]["body"], f"src/ui_golden/{key}.json is stale"
