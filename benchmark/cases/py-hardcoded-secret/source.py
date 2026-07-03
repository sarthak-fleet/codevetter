# Case: Hardcoded cloud API key committed to source.
import os
import requests


def fetch_billing(account_id: str) -> dict:
    # BUG: a live AWS access key is hardcoded in source instead of being read
    # from a secret manager or environment variable. Anyone with repo access
    # can impersonate this account.
    aws_access_key_id = "AKIAIOSFODNN7EXAMPLE"
    aws_secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    region = "us-east-1"

    url = f"https://billing.example.com/{account_id}"
    resp = requests.get(
        url,
        headers={
            "X-Aws-Key": aws_access_key_id,
            "X-Aws-Secret": aws_secret_access_key,
            "X-Aws-Region": region,
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()
