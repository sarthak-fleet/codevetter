// Case: Type-safety bypass via `as any` that hides a real shape mismatch.
interface Order {
  id: string;
  total: number;
  items: string[];
}

interface Refund {
  id: string;
  amount: number;
  reason: string;
}

// BUG: the caller casts the payload to `any` so the compiler cannot catch that
// a Refund is being treated as an Order. At runtime `items` is undefined and
// the .length access throws, or worse, silently corrupts downstream totals.
export function processOrder(payload: unknown): number {
  const order = payload as any as Order;
  return order.total + order.items.length;
}
