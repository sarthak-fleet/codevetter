// Case: SQL injection via string concatenation in a TypeScript query builder.
import { db } from './db';

interface User {
  id: number;
  email: string;
}

export async function findUserByEmail(emailInput: string): Promise<User | null> {
  // BUG: user-controlled emailInput is concatenated directly into the SQL
  // string, allowing an attacker to break out of the quoted value and append
  // arbitrary SQL (e.g. "' OR '1'='1").
  const sql = `SELECT id, email FROM users WHERE email = '${emailInput}' LIMIT 1`;
  const rows = await db.query<User>(sql);
  return rows[0] ?? null;
}
