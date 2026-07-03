// Case: Hardcoded database credentials in a TypeScript service config.
export const dbConfig = {
  host: 'db.prod.internal',
  port: 5432,
  user: 'admin',
  // BUG: the production database password is committed in plaintext.
  password: 'P@ssw0rd-prod-2024!',
  database: 'orders',
};

export async function connect() {
  const url = `postgres://${dbConfig.user}:${dbConfig.password}@${dbConfig.host}:${dbConfig.port}/${dbConfig.database}`;
  return fetch(url);
}
