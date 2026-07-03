// Case: Missing await on a rejected promise swallows an error.
import { deleteSession } from './session';

export async function logout(userId: string): Promise<void> {
  // BUG: deleteSession returns a promise but is not awaited. If it rejects,
  // the rejection becomes an unhandled promise rejection and logout resolves
  // as if the session were deleted, leaving stale sessions behind.
  deleteSession(userId);
  console.log('user logged out');
}
