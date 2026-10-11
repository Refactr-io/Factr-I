"""Factr Desktop (Chat GUI) uninstaller: removes only GUI state — built Electron artifacts, the packaged
app, and the desktop's own ``userData`` — never agent source, venv, config, sessions or .env."""

import base64
import json
import os
import shutil
import sys
from pathlib import Path

from factr_constants import desktop_app_dir, get_factr_home

from factr_backend.colors import Colors, color


def _logger(mark: str, col: str):
    return lambda msg: print(f"{color(mark, col)} {msg}")


log_info, log_success = _logger("→", Colors.CYAN), _logger("✓", Colors.GREEN)
log_warn = _logger("⚠", Colors.YELLOW)


def _env_dir(var: str, fallback: Path) -> Path:
    """``Path($var)`` when the env var is set, else *fallback*."""
    return Path(value) if (value := os.environ.get(var)) else fallback


def explicit_targets() -> "list[dict] | None":
    """The targets the desktop's uninstall confirmation listed, when the desktop launched this run.

    The desktop passes them as base64 JSON in ``FACTR_UNINSTALL_TARGETS_B64`` (``{"targets": [{"kind",
    "path"}]}``, kinds: app, artifact, agent, userdata, config). When set, the uninstall deletes those paths
    and nothing else: no standard install locations, no default userData. ``None`` for a CLI run; an
    unreadable value is ``[]`` (delete nothing) rather than a fallback to the defaults."""
    raw = os.environ.get("FACTR_UNINSTALL_TARGETS_B64")
    if not raw:
        return None
    try:
        targets = json.loads(base64.b64decode(raw).decode("utf-8")).get("targets") or []
    except Exception:
        log_warn("The desktop uninstall targets could not be read; nothing will be deleted.")
        return []
    return [t for t in targets if isinstance(t, dict) and isinstance(t.get("path"), str)
            and isinstance(t.get("kind"), str) and os.path.isabs(t["path"])]


def explicit_paths(*kinds: str) -> "list[Path] | None":
    """Paths of the given kinds from :func:`explicit_targets`, or ``None`` for a CLI run."""
    targets = explicit_targets()
    if targets is None:
        return None
    return [Path(t["path"]) for t in targets if t["kind"] in kinds]


def _protected_dirs() -> "list[Path]":
    """System and home folders an uninstall must never delete, nor any folder containing one."""
    home = Path.home()
    if sys.platform == "win32":
        drive = Path(home.anchor or "C:\\")
        extra = [os.environ.get(v) for v in ("APPDATA", "LOCALAPPDATA", "ProgramFiles")]
        return [drive, drive / "Users", drive / "Windows", home, home / "AppData", home / "AppData" / "Roaming",
                home / "AppData" / "Local", home / "Documents", home / "Desktop"] + [Path(v) for v in extra if v]
    return [Path(p) for p in ("/", "/Applications", "/Users", "/System", "/Library", "/Volumes", "/home", "/usr",
                              "/bin", "/etc", "/opt", "/private", "/tmp", "/var")] + [
        home, home / "Applications", home / "Library", home / "Library" / "Application Support",
        home / "Documents", home / "Desktop", home / ".config", home / ".local", home / ".local" / "share"]


def refusal_reason(path: Path) -> "str | None":
    """Why ``path`` must not be deleted, or ``None``. Refuses relative paths and any path that is (or
    contains) a system or home folder, checked both as given and with symlinks resolved."""
    if not path.is_absolute():
        return "not an absolute path"
    candidates = {Path(os.path.normpath(path))}
    try:
        candidates.add(path.resolve())
    except OSError:
        pass
    for candidate in candidates:
        for guarded in _protected_dirs():
            try:
                guarded_norm = Path(os.path.normpath(guarded))
            except (TypeError, ValueError):
                continue
            if candidate == guarded_norm or candidate in guarded_norm.parents:
                return f"it is (or contains) the protected folder {guarded_norm}"
    return None


def desktop_userdata_dir() -> Path:
    """Electron ``app.getPath('userData')`` for the app named "Factr-I" on each platform (GUI-only state).

    The desktop names the folder its running copy really uses (``FACTR_UNINSTALL_USERDATA`` for the summary
    probe, the confirmed targets for a run): an overridden or non-default userData is never guessed."""
    if confirmed := explicit_paths("userdata"):
        return confirmed[0]
    if override := os.environ.get("FACTR_UNINSTALL_USERDATA"):
        return Path(override)
    home = Path.home()
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / "Factr-I"
    if sys.platform == "win32":
        return _env_dir("APPDATA", home / "AppData" / "Roaming") / "Factr-I"
    return _env_dir("XDG_CONFIG_HOME", home / ".config") / "Factr-I"


def source_built_gui_artifacts(factr_home: Path) -> "list[Path]":
    """GUI build artifacts produced by ``factr desktop`` inside the checkout (same ``factr-backend/`` layout
    install.sh uses). The Python agent runs from source + venv and never needs the Electron build output or
    node_modules (the workspace-root node_modules only carries Electron, ~200MB)."""
    agent_root = factr_home / "factr-backend"
    desktop_dir = desktop_app_dir(agent_root)
    return [desktop_dir / "dist", desktop_dir / "release", desktop_dir / "node_modules",
            agent_root / "node_modules", factr_home / "desktop-build-stamp.json"]


def packaged_gui_app_paths() -> "list[Path]":
    """Standard install locations of the packaged desktop distributable for the current OS. Every candidate
    is returned; the caller filters to those that exist. Never globs system-wide — only the well-known
    electron-builder output locations for the "Factr-I" product.

    A desktop-launched run returns none: its cleanup script removes the RUNNING copy's bundle (the one the
    confirmation showed) after this process exits, never a copy at a standard location."""
    if explicit_targets() is not None:
        return []
    home = Path.home()
    if sys.platform == "darwin":
        return [Path("/Applications/Factr-I.app"), home / "Applications" / "Factr-I.app"]
    if sys.platform == "win32":
        local_base = _env_dir("LOCALAPPDATA", home / "AppData" / "Local")
        # NSIS per-user install (perMachine=false), an older/alternate layout, NSIS per-machine (needs admin).
        program_files = os.environ.get("ProgramFiles")
        return [local_base / "Programs" / "Factr-I", local_base / "factr-desktop"] + (
            [Path(program_files) / "Factr-I"] if program_files else [])
    # Linux: an AppImage lives wherever the user put it and deb/rpm files belong to the package manager
    # (see the hint in ``uninstall_gui``), so only the desktop entry + hicolor icons are cleaned here.
    from factr_backend.linux_desktop_entry import desktop_entry_path
    data_base = _env_dir("XDG_DATA_HOME", home / ".local" / "share")
    icons = data_base / "icons" / "hicolor"
    # "scalable" plus every fixed-size dir the installer may have written (panel sizes + older native copies).
    return [desktop_entry_path(), data_base / "applications" / "Factr-I.desktop"] + [
        icons / size / "apps" / "factr.png"
        for size in ("scalable", "24x24", "32x32", "48x48", "256x256", "512x512", "1024x1024")]


def agent_is_installed(factr_home: Path) -> bool:
    """True when a usable Python agent install exists under FACTR_CONFIG_HOME (gates the desktop UI's options).
    Package source or a venv alone is enough — a source checkout without a venv is still "the agent is here"."""
    return any((factr_home / "factr-backend" / sub).is_dir() for sub in ("factr_backend", "venv", ".venv"))


def gui_is_installed(factr_home: Path) -> bool:
    """Return True when any desktop GUI artifact exists (built or packaged)."""
    return any(p.exists() for p in (*source_built_gui_artifacts(factr_home), *packaged_gui_app_paths(), desktop_userdata_dir()))


def gui_install_summary(factr_home: "Path | None" = None) -> dict:
    """JSON-serializable snapshot of what's installed, for the desktop UI to render via IPC."""
    home: Path = factr_home if factr_home is not None else get_factr_home()
    userdata = desktop_userdata_dir()
    return {"factr_home": str(home), "agent_installed": agent_is_installed(home),
            "gui_installed": gui_is_installed(home),
            "source_built_artifacts": [str(p) for p in source_built_gui_artifacts(home) if p.exists()],
            "packaged_app_paths": [str(p) for p in packaged_gui_app_paths() if p.exists()],
            "userdata_dir": str(userdata), "userdata_exists": userdata.exists(), "platform": sys.platform}


def _remove_path(path: Path) -> bool:
    """Remove a file or directory tree. Returns True when something was removed. A symlink is unlinked,
    never followed; a protected folder (see :func:`refusal_reason`) is refused."""
    if (reason := refusal_reason(path)) is not None:
        log_warn(f"Not removing {path}: {reason}")
        return False
    try:
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.is_dir():
            shutil.rmtree(path)
        else:
            return False
        return True
    except Exception as e:
        log_warn(f"Could not remove {path}: {e}")
        return False


def uninstall_gui(factr_home: "Path | None" = None, *, remove_userdata: bool = True) -> "list[Path]":
    """Remove the desktop GUI's artifacts, leaving the agent + user data intact."""
    home: Path = factr_home if factr_home is not None else get_factr_home()
    removed: list[Path] = []

    def _remove_existing(paths) -> bool:
        """Remove every existing path; True when at least one existed."""
        found = False
        for path in (p for p in paths if p.exists()):
            found = True
            if _remove_path(path):
                log_success(f"Removed {path}")
                removed.append(path)
        return found
    confirmed_artifacts = explicit_paths("artifact")
    log_info("Removing built GUI artifacts (renderer, release, node_modules)...")
    # A desktop-launched run removes only the artifacts its confirmation listed.
    _remove_existing(source_built_gui_artifacts(home) if confirmed_artifacts is None else confirmed_artifacts)
    log_info("Removing installed desktop app...")
    if not _remove_existing(packaged_gui_app_paths()):
        log_info("No packaged desktop app found in standard locations")
    # A desktop-launched run removes the userData its confirmation listed, and none when it listed none.
    confirmed_userdata = explicit_paths("userdata")
    userdata_dirs = [desktop_userdata_dir()] if confirmed_userdata is None else confirmed_userdata
    if remove_userdata and any(p.exists() for p in userdata_dirs):
        log_info("Removing desktop app data (Electron userData)...")
        _remove_existing(userdata_dirs)
    if not removed:
        log_info("No desktop GUI artifacts found to remove")
    if sys.platform.startswith("linux"):
        # The desktop entry was removed above but the menu caches still list it; reindex so Factr
        # disappears from the launcher.
        try:
            from factr_backend.linux_desktop_entry import desktop_entry_path, refresh_desktop_databases
            entry = desktop_entry_path()
            if entry in removed:
                for tool in refresh_desktop_databases(entry.parent):
                    log_success(f"Refreshed the application menu cache ({tool})")
        except Exception as e:
            log_warn(f"Could not refresh the application menu cache: {e}")
        log_info("If you installed the desktop via a .deb / .rpm package, remove it with your package manager "
                 "(e.g. 'sudo apt remove factr' or 'sudo dnf remove factr'). AppImage builds are a single "
                 "file you can delete from wherever you saved it.")
    return removed
