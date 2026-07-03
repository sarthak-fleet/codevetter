# Case: Server-side request forgery via a user-supplied URL.
import requests


def fetch_preview(image_url: str) -> bytes:
    # BUG: the server fetches whatever URL the user supplies, with no scheme,
    # host, or private-range restrictions. An attacker can point this at
    # http://169.254.169.254/latest/meta-data/ to read cloud metadata creds or
    # at internal services to pivot.
    resp = requests.get(image_url, timeout=5)
    resp.raise_for_status()
    return resp.content
