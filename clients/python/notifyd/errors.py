from __future__ import annotations

from typing import Any, Optional


class NotifydError(Exception):
    """Raised for every non-2xx answer from notifyd.

    `status` is the HTTP status, `message` the server's `error` field when present,
    `details` the raw JSON body (or text) for logging.
    """

    def __init__(self, status: int, message: str, details: Any = None, retry_after: Optional[float] = None):
        super().__init__(f"notifyd {status}: {message}")
        self.status = status
        self.message = message
        self.details = details
        self.retry_after = retry_after

    @property
    def is_rate_limited(self) -> bool:
        return self.status == 429

    @property
    def is_retryable(self) -> bool:
        return self.status == 429 or self.status >= 500
