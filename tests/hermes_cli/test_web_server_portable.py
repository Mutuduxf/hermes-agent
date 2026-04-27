"""Tests for the Windows-USB portable-mode hooks in start_server.

Verifies that when ``HERMES_PORTABLE=1`` the dashboard:

* writes its session token + bound URL atomically to
  ``$HERMES_HOME/cache/dashboard-token.txt`` so the GUI launcher can
  pick them up;
* registers an atexit handler that wipes the token file (best-effort
  WAL flush is exercised in :mod:`tests.hermes_state`).
"""

from __future__ import annotations

import atexit
import sys
import types
from pathlib import Path


def _stub_uvicorn(monkeypatch):
    fake = types.ModuleType("uvicorn")
    fake.run = lambda *a, **kw: None
    monkeypatch.setitem(sys.modules, "uvicorn", fake)


def test_portable_mode_writes_token_file(tmp_path, monkeypatch):
    home = tmp_path / "data"
    home.mkdir()
    monkeypatch.setenv("HERMES_HOME", str(home))
    monkeypatch.setenv("HERMES_PORTABLE", "1")

    _stub_uvicorn(monkeypatch)
    from hermes_cli import web_server as ws

    registered = []
    monkeypatch.setattr(atexit, "register", lambda fn: registered.append(fn) or fn)

    ws.start_server(host="127.0.0.1", port=12345, open_browser=False)

    token_path = home / "cache" / "dashboard-token.txt"
    assert token_path.exists(), "portable mode must write the dashboard token file"
    payload = token_path.read_text(encoding="utf-8")
    assert "url=http://127.0.0.1:12345" in payload
    assert "token=" in payload
    assert "pid=" in payload

    # The atexit handler must clean up the token file when called.
    assert registered, "portable mode must register a shutdown handler"
    for handler in registered:
        handler()
    assert not token_path.exists(), "shutdown handler must wipe the token file"


def test_non_portable_mode_does_not_write_token(tmp_path, monkeypatch):
    home = tmp_path / "data"
    home.mkdir()
    monkeypatch.setenv("HERMES_HOME", str(home))
    monkeypatch.delenv("HERMES_PORTABLE", raising=False)

    _stub_uvicorn(monkeypatch)
    from hermes_cli import web_server as ws

    monkeypatch.setattr(atexit, "register", lambda fn: fn)

    ws.start_server(host="127.0.0.1", port=12346, open_browser=False)

    assert not (home / "cache" / "dashboard-token.txt").exists()
