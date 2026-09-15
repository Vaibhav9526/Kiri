export function CountdownOverlay({ value }: { value?: number | null }) {
  return (
    <main className="countdown-overlay" role="status" aria-live="assertive">
      <span>{value ?? '…'}</span>
      <small>Recording starts shortly</small>
    </main>
  );
}
