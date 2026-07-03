// Case: Unchecked arithmetic that panics on overflow and can wrap balances.
pub struct Account {
    pub balance: u64,
}

impl Account {
    pub fn credit(&mut self, amount: u64) {
        // BUG: in debug builds this panics on overflow; in release builds it
        // wraps silently, so crediting a huge amount can roll the balance back
        // to a small value. Use checked_add / saturating_add explicitly.
        self.balance += amount;
    }

    pub fn debit(&mut self, amount: u64) -> u64 {
        // BUG: subtraction underflow panics in debug and wraps in release,
        // letting an over-debit produce a huge balance.
        self.balance -= amount;
        self.balance
    }
}
