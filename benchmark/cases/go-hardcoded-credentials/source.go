// Case: Hardcoded database credentials in a Go service.
package db

const (
	// BUG: production database credentials are committed in plaintext.
	dsnUser = "billing_admin"
	dsnPass = "supersecret-prod-2024"
	dsnHost = "10.0.0.5:5432"
	dsnName = "billing"
)

func DSN() string {
	return "postgres://" + dsnUser + ":" + dsnPass + "@" + dsnHost + "/" + dsnName
}
