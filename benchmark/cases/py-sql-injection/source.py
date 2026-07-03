# Case: SQL injection via f-string in a Python ORM call.
import sqlite3


def search_products(db: sqlite3.Connection, name: str) -> list:
    # BUG: name is interpolated into the SQL string with an f-string. A value
    # like "x' UNION SELECT password FROM users--" appends an arbitrary query.
    cursor = db.execute(f"SELECT id, name FROM products WHERE name LIKE '%{name}%'")
    return cursor.fetchall()
