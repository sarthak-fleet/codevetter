// Case: Data race on a shared map accessed from concurrent goroutines
// without synchronization.
package cache

import (
	"sync"
)

type Cache struct {
	mu   sync.Mutex
	data map[string]string
}

func NewCache() *Cache {
	return &Cache{data: make(map[string]string)}
}

// Get is called from many goroutines but reads the map without holding mu.
func (c *Cache) Get(key string) (string, bool) {
	v, ok := c.data[key] // BUG: unsynchronized concurrent map read
	return v, ok
}

// Set holds the lock, but concurrent reads above still race with writes here.
func (c *Cache) Set(key, value string) {
	c.mu.Lock()
	c.data[key] = value
	c.mu.Unlock()
}
