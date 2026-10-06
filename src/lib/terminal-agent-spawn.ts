const inFlightSpawns = new Map<string, Promise<unknown>>()

export function startTerminalAgentOnce<T>(
  terminalId: string,
  start: () => Promise<T>
): { promise: Promise<T>; owner: boolean } {
  const existing = inFlightSpawns.get(terminalId) as Promise<T> | undefined
  if (existing) return { promise: existing, owner: false }

  const promise = start().finally(() => {
    if (inFlightSpawns.get(terminalId) === promise) {
      inFlightSpawns.delete(terminalId)
    }
  })
  inFlightSpawns.set(terminalId, promise)
  return { promise, owner: true }
}
