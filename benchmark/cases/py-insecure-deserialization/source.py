# Case: Insecure deserialization of untrusted pickle data.
import pickle
import base64


def load_state(blob: str) -> dict:
    # BUG: pickle can execute arbitrary code during deserialization. Decoding
    # and unpickling a user-supplied blob lets an attacker run any payload on
    # the server. Use JSON or a restricted schema instead.
    raw = base64.b64decode(blob)
    return pickle.loads(raw)
