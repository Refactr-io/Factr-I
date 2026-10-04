

def test_tui_finds_bundled_entry_js(tmp_path):
    """_find_bundled_tui finds entry.js bundled in the package."""
    tui_dist = tmp_path / "factr_backend" / "tui_dist"
    tui_dist.mkdir(parents=True)
    entry = tui_dist / "entry.js"
    entry.write_text("// bundled TUI", encoding="utf-8")

    from factr_backend.main_tui_launch import _find_bundled_tui
    result = _find_bundled_tui(factr_backend_dir=tmp_path / "factr_backend")
    assert result is not None
    assert result.name == "entry.js"


