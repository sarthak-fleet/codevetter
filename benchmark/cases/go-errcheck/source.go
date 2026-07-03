// Case: Ignored error return from a write that can fail.
package writer

import (
	"os"
)

func SaveConfig(path string, contents []byte) {
	// BUG: WriteFile's error is discarded. If the disk is full, permissions
	// are wrong, or the path is invalid, the failure is silently swallowed and
	// callers proceed as if the config was saved.
	os.WriteFile(path, contents, 0o600)
}
