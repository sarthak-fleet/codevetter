# Case: Using MD5 to hash passwords.
import hashlib


def hash_password(password: str) -> str:
    # BUG: MD5 is cryptographically broken and unsalted, so identical passwords
    # produce identical hashes that are trivially cracked via rainbow tables.
    return hashlib.md5(password.encode()).hexdigest()


def verify_password(password: str, stored: str) -> bool:
    return hash_password(password) == stored
