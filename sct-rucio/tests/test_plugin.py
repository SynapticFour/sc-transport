from unittest.mock import patch

import pytest

from sct_rucio.plugin import SctTransferTool


class DummyResp:
    def __init__(self, status_code=200, payload=None):
        self.status_code = status_code
        self._payload = payload or {}

    def raise_for_status(self):
        if self.status_code >= 400 and self.status_code != 404:
            raise RuntimeError(f"http {self.status_code}")

    def json(self):
        return self._payload


def test_submit_returns_ids():
    with patch("sct_rucio.plugin.requests.post") as mock_post:
        mock_post.return_value = DummyResp(payload={"transfer_id": "abc"})
        tool = SctTransferTool("localhost")
        ids = tool.submit(["sct://host/path"], {"priority": 3})
        assert ids == ["abc"]


def test_submit_propagates_http_error():
    with patch("sct_rucio.plugin.requests.post") as mock_post:
        mock_post.return_value = DummyResp(status_code=500)
        tool = SctTransferTool("localhost")
        with pytest.raises(RuntimeError, match="http 500"):
            tool.submit(["sct://host/path"], {"priority": 3})


@pytest.mark.parametrize(
    ("daemon_status", "expected_state"),
    [
        ("queued", "SUBMITTED"),
        ("active", "SUBMITTING"),
        ("completed", "DONE"),
        ("failed", "FAILED"),
        ("cancelled", "CANCELED"),
        ("unknown", "SUBMITTED"),
    ],
)
def test_query_maps_all_states(daemon_status, expected_state):
    with patch("sct_rucio.plugin.requests.get") as mock_get:
        mock_get.return_value = DummyResp(payload={"status": daemon_status})
        tool = SctTransferTool("localhost")
        states = tool.query(["abc"])
        assert states["abc"]["state"] == expected_state


def test_query_404_uses_default_result():
    with patch("sct_rucio.plugin.requests.get") as mock_get:
        mock_get.return_value = DummyResp(status_code=404)
        tool = SctTransferTool("localhost")
        states = tool.query(["abc"], default_result={"state": "MISSING"})
        assert states["abc"]["state"] == "MISSING"


def test_query_propagates_timeout():
    with patch("sct_rucio.plugin.requests.get", side_effect=TimeoutError("timed out")):
        tool = SctTransferTool("localhost")
        with pytest.raises(TimeoutError, match="timed out"):
            tool.query(["abc"])


def test_cancel_and_update_priority_call_rest_endpoints():
    with patch("sct_rucio.plugin.requests.delete") as mock_delete, patch(
        "sct_rucio.plugin.requests.patch"
    ) as mock_patch:
        mock_delete.return_value = DummyResp(status_code=204)
        mock_patch.return_value = DummyResp(status_code=204)
        tool = SctTransferTool("localhost")
        assert tool.cancel(["a", "b"]) is True
        assert tool.update_priority("a", 1) is True
        assert mock_delete.call_count == 2
        mock_patch.assert_called_once()
