import pytest
from unittest.mock import patch, MagicMock

from sct_rucio.plugin import SctRSEProtocol

PROTO_ATTR = {"hostname": "localhost", "port": 9410, "prefix": "/data/"}


def make_proto():
    return SctRSEProtocol(PROTO_ATTR, rse_settings={})


def test_get_calls_sct_receive():
    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0, stderr=b"")
        make_proto().get("/atlas/file.root", "/tmp/file.root")
        cmd = mock_run.call_args[0][0]
        assert "receive" in cmd
        assert "--resume" in cmd


def test_put_calls_sct_send():
    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0, stderr=b"")
        make_proto().put("/tmp/file.root", "/atlas/file.root")
        cmd = mock_run.call_args[0][0]
        assert "send" in cmd
        assert "--compression" in cmd


def test_get_raises_on_nonzero_returncode():
    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=1, stderr=b"connection refused")
        with pytest.raises(RuntimeError, match="sct receive failed"):
            make_proto().get("/file", "/tmp/file")


def test_put_raises_on_nonzero_returncode():
    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=2, stderr=b"timeout")
        with pytest.raises(RuntimeError, match="sct send failed"):
            make_proto().put("/tmp/file", "/remote/file")
