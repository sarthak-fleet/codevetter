# Case: Path traversal in a file download handler.
import os


def read_report(report_name: str) -> str:
    base_dir = "/var/app/reports"
    # BUG: report_name is joined directly into the filesystem path without
    # normalization or containment checks. A request like
    # "../../etc/passwd" escapes base_dir and reads arbitrary files.
    full_path = os.path.join(base_dir, report_name)
    with open(full_path, "r", encoding="utf-8") as fh:
        return fh.read()
