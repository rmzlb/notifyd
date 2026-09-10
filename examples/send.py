# pip install notifyd-sdk
import os
import time

from notifyd import Notifyd, NotifydError

nd = Notifyd(os.environ["NOTIFYD_URL"], api_key=os.environ["NOTIFYD_API_KEY"])

nd.upsert_subscriber("user-42", email="alice@example.com", first_name="Alice", timezone="Europe/Paris")

result = nd.send(
    channels=["email", "in_app"],
    subscriber_id="user-42",
    subject="Your order shipped",
    body="Hi {{first_name}}, parcel {{parcel}} is on its way.",
    vars={"first_name": "Alice", "parcel": "FR-2041"},
    idempotency_key="order-2041-shipped",  # safe to retry: the same key never sends twice
)

time.sleep(1)
for job_id in result["job_ids"]:
    try:
        job = nd.get_job(job_id)
        print(f"{job['channel']}: {job['status']} via {job.get('provider') or '-'} after {job['attempts']} attempt(s)")
    except NotifydError as e:
        print(e.status, e.message)
