// Per-viewer view preferences. Storage may be unavailable (private mode,
// cleared site data); then the defaults simply come back next time.

export function load<T>(key: string, parse: (raw: string) => T | undefined, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return (raw !== null && parse(raw)) || fallback;
  } catch {
    return fallback;
  }
}

export function store(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not persisted.
  }
}
