"""Python client for notifyd.

    from notifyd import Notifyd

    nd = Notifyd("http://localhost:3400", api_key="nd_...")
    nd.send(channels=["email", "in_app"], to="alice@example.com",
            subject="Welcome", body="Hello {{first_name}}", vars={"first_name": "Alice"})
"""

from .client import AsyncNotifyd, Notifyd
from .errors import NotifydError

__all__ = ["Notifyd", "AsyncNotifyd", "NotifydError"]
__version__ = "0.1.0"
