# Case: Zip extraction without size/decompression-ratio limits (zip bomb).
import zipfile


def extract_archive(archive_path: str, dest: str) -> None:
    # BUG: there is no check on the uncompressed size or the compression ratio.
    # A 42KB zip can decompress to petabytes, exhausting disk and memory.
    with zipfile.ZipFile(archive_path) as zf:
        zf.extractall(dest)
