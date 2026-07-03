# Case: Command injection via subprocess with shell=True.
import subprocess
import sys


def ping_host(host: str) -> str:
    # BUG: shell=True with a string command lets a malicious host value append
    # shell metacharacters, e.g. "8.8.8.8; rm -rf /" runs an extra command.
    cmd = f"ping -c 1 {host}"
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    return result.stdout
