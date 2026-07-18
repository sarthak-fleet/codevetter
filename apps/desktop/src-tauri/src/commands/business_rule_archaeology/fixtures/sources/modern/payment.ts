export function approvePayment(balance: number, amount: number) {
  if (amount > 0 && balance >= amount) {
    return { status: 'approved', remaining: balance - amount };
  }
  return { status: 'rejected', remaining: balance };
}
