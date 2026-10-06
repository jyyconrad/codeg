export interface LatestSerialQueueContext {
  hasNewerPending: () => boolean
}

interface LatestSerialQueueHandlers {
  onLatestSuccess?: (context: LatestSerialQueueContext) => void | Promise<void>
  onLatestFailure?: (
    error: unknown,
    context: LatestSerialQueueContext
  ) => void | Promise<void>
  onFailure?: (error: unknown) => void
}

export class LatestSerialQueue<Key> {
  private readonly chains = new Map<Key, Promise<void>>()
  private readonly sequences = new Map<Key, number>()

  hasPending(key: Key): boolean {
    return this.chains.has(key)
  }

  enqueue(
    key: Key,
    task: () => Promise<void>,
    handlers: LatestSerialQueueHandlers = {}
  ): Promise<void> {
    const sequence = (this.sequences.get(key) ?? 0) + 1
    this.sequences.set(key, sequence)
    const prior = this.chains.get(key) ?? Promise.resolve()
    const context: LatestSerialQueueContext = {
      hasNewerPending: () => (this.sequences.get(key) ?? 0) > sequence,
    }
    const run = prior
      .catch(() => {})
      .then(task)
      .then(async () => {
        if (this.sequences.get(key) === sequence) {
          await handlers.onLatestSuccess?.(context)
        }
      })
      .catch(async (error: unknown) => {
        handlers.onFailure?.(error)
        if (this.sequences.get(key) === sequence) {
          await handlers.onLatestFailure?.(error, context)
        }
      })
      .finally(() => {
        if (this.chains.get(key) === run) this.chains.delete(key)
      })
    this.chains.set(key, run)
    return run
  }
}
