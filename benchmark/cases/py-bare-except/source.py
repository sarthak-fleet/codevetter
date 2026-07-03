# Case: Bare except swallows all errors including KeyboardInterrupt.
import json


def parse_config(raw: str) -> dict:
    try:
        return json.loads(raw)
    except:  # BUG: catches BaseException, hiding bugs, KeyboardInterrupt, and
            # SystemExit, and returns an empty dict so callers never learn the
            # config failed to parse.
        return {}


def load_settings(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as fh:
        return parse_config(fh.read())
