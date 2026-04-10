try:
    import requests
except Exception:  # pragma: no cover
    class _MissingRequests:
        def _missing(self, *args, **kwargs):
            raise RuntimeError("requests dependency is required for sct-rucio plugin")

        get = _missing
        post = _missing
        patch = _missing
        delete = _missing

    requests = _MissingRequests()

try:
    from rucio.transfertool.transfertool import TransferTool
except Exception:  # pragma: no cover
    class TransferTool:  # lightweight fallback for local linting/imports
        external_name = "sct"


class SctTransferTool(TransferTool):
    """
    sc-transport transfer tool plugin for Rucio.
    Calls sct-daemon REST API to submit and monitor transfers.
    """

    external_name = "sct"

    def __init__(self, external_host, **kwargs):
        self.api_base = f"http://{external_host}:7273/v1"

    def submit(self, transfers, job_params, timeout=None):
        transfer_ids = []
        for t in transfers:
            payload = {
                "source": getattr(t, "source_url", str(t)),
                "destination": getattr(t, "dest_url", "/tmp"),
                "priority": int(job_params.get("priority", 5)),
            }
            r = requests.post(f"{self.api_base}/transfer", json=payload, timeout=timeout or 30)
            r.raise_for_status()
            transfer_ids.append(r.json()["transfer_id"])
        return transfer_ids

    def query(self, transfer_ids, default_result=None):
        out = {}
        for transfer_id in transfer_ids:
            r = requests.get(f"{self.api_base}/transfer/{transfer_id}", timeout=10)
            if r.status_code == 404:
                out[transfer_id] = default_result or {"state": "SUBMISSION_FAILED"}
                continue
            r.raise_for_status()
            status = r.json()["status"]
            state = {
                "queued": "SUBMITTED",
                "active": "SUBMITTING",
                "completed": "DONE",
                "failed": "FAILED",
                "cancelled": "CANCELED",
            }.get(status, "SUBMITTED")
            out[transfer_id] = {"state": state}
        return out

    def cancel(self, transfer_ids, timeout=None):
        for transfer_id in transfer_ids:
            requests.delete(f"{self.api_base}/transfer/{transfer_id}", timeout=timeout or 30).raise_for_status()
        return True

    def update_priority(self, transfer_id, priority):
        requests.patch(
            f"{self.api_base}/transfer/{transfer_id}",
            json={"priority": int(priority)},
            timeout=30,
        ).raise_for_status()
        return True
