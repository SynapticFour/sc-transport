"""
Integration-Test: echter sct-daemon + echter Transfer.
Läuft nur wenn SCT_DAEMON_HOST gesetzt ist (CI mit Docker Compose).
"""

import hashlib
import os
import pathlib
import tempfile
import time

import pytest

from sct_rucio.plugin import SctTransferTool

DAEMON_HOST = os.environ.get("SCT_DAEMON_HOST", "")
TRANSFER_DIR = os.environ.get("SCT_TRANSFER_DIR", "")
pytestmark = pytest.mark.skipif(
    not DAEMON_HOST,
    reason="SCT_DAEMON_HOST not set — skipping integration tests",
)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def transfer_workdir():
    if TRANSFER_DIR:
        root = pathlib.Path(TRANSFER_DIR)
        root.mkdir(parents=True, exist_ok=True)
        return root, False
    tmp = tempfile.TemporaryDirectory()
    return pathlib.Path(tmp.name), True


def test_full_transfer_small_file():
    """Überträgt 1 MB Testdatei und prüft Integrität."""
    workdir, cleanup = transfer_workdir()
    try:
        src = workdir / "test_1mb.bin"
        src.write_bytes(os.urandom(1024 * 1024))
        src_hash = sha256(src)

        tool = SctTransferTool(DAEMON_HOST)

        class Transfer:
            source_url = f"file://{src}"
            dest_url = str(workdir / "recv_1mb.bin")

        ids = tool.submit([Transfer()], {"priority": 5}, timeout=30)
        assert len(ids) == 1

        for _ in range(30):
            states = tool.query(ids)
            if states[ids[0]]["state"] in ("DONE", "FAILED"):
                break
            time.sleep(1)

        assert states[ids[0]]["state"] == "DONE"
        recv = workdir / "recv_1mb.bin"
        assert recv.exists(), "Output file missing"
        assert sha256(recv) == src_hash, "Checksum mismatch"
    finally:
        if cleanup:
            import shutil

            shutil.rmtree(workdir, ignore_errors=True)


def test_full_transfer_large_file():
    """Überträgt 100 MB Testdatei."""
    workdir, cleanup = transfer_workdir()
    try:
        src = workdir / "test_100mb.bin"
        src.write_bytes(os.urandom(100 * 1024 * 1024))
        src_hash = sha256(src)
        tool = SctTransferTool(DAEMON_HOST)

        class Transfer:
            source_url = f"file://{src}"
            dest_url = str(workdir / "recv_100mb.bin")

        ids = tool.submit([Transfer()], {"priority": 3}, timeout=120)
        for _ in range(120):
            states = tool.query(ids)
            if states[ids[0]]["state"] in ("DONE", "FAILED"):
                break
            time.sleep(1)
        assert states[ids[0]]["state"] == "DONE"
        assert sha256(workdir / "recv_100mb.bin") == src_hash
    finally:
        if cleanup:
            import shutil

            shutil.rmtree(workdir, ignore_errors=True)
