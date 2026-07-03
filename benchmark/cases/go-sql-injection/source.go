// Case: SQL injection via fmt.Sprintf in a Go database query.
package store

import (
	"database/sql"
	"fmt"
)

func FindByEmail(db *sql.DB, email string) (*sql.Row, error) {
	// BUG: email is interpolated into the query string with fmt.Sprintf instead
	// of using parameterized placeholders, allowing SQL injection.
	q := fmt.Sprintf("SELECT id, email FROM users WHERE email = '%s'", email)
	row := db.QueryRow(q)
	return row, row.Err()
}
