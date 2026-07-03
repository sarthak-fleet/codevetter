// Case: Nil-pointer dereference when a lookup returns no value.
package user

type User struct {
	ID    int
	Email string
}

func FindUser(users map[int]*User, id int) string {
	// BUG: FindUser returns the dereferenced Email without checking that the
	// map lookup returned a non-nil pointer. A missing id causes a nil pointer
	// dereference and a process crash.
	u := users[id]
	return u.Email
}
