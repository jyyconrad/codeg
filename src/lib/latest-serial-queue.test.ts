import { describe, expect, it, vi } from "vitest"
import { LatestSerialQueue } from "@/lib/latest-serial-queue"

function deferred() {
  let resolve!: () => void
  let reject!: (error: unknown) => void
  const promise = new Promise<void>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

describe("LatestSerialQueue", () => {
  it("serializes a key and only publishes completion for its latest request", async () => {
    const queue = new LatestSerialQueue<string>()
    const first = deferred()
    const second = deferred()
    const started: string[] = []
    const latest = vi.fn()

    const firstDone = queue.enqueue(
      "codex",
      async () => {
        started.push("first")
        await first.promise
      },
      { onLatestSuccess: latest }
    )
    const secondDone = queue.enqueue(
      "codex",
      async () => {
        started.push("second")
        await second.promise
      },
      { onLatestSuccess: latest }
    )

    await vi.waitFor(() => expect(started).toEqual(["first"]))
    first.resolve()
    await firstDone
    await vi.waitFor(() => expect(started).toEqual(["first", "second"]))
    expect(latest).not.toHaveBeenCalled()
    second.resolve()
    await secondDone
    expect(latest).toHaveBeenCalledTimes(1)
  })

  it("lets latest failure refresh ignore itself without releasing its chain", async () => {
    const queue = new LatestSerialQueue<string>()
    let draft = "terminal"
    const backendAfterExternalUpdate = "acp"

    await queue.enqueue(
      "codex",
      async () => {
        throw new Error("save failed")
      },
      {
        onLatestFailure: (_error, context) => {
          expect(queue.hasPending("codex")).toBe(true)
          expect(context.hasNewerPending()).toBe(false)
          draft = backendAfterExternalUpdate
        },
      }
    )

    expect(draft).toBe("acp")
  })

  it("keeps B queued while A's authoritative refresh is still running", async () => {
    const queue = new LatestSerialQueue<string>()
    const refreshA = deferred()
    const saveB = deferred()
    const started: string[] = []

    const doneA = queue.enqueue(
      "codex",
      async () => {
        started.push("save-a")
      },
      {
        onLatestSuccess: async (context) => {
          started.push("refresh-a")
          await refreshA.promise
          expect(context.hasNewerPending()).toBe(true)
        },
      }
    )
    await vi.waitFor(() => expect(started).toEqual(["save-a", "refresh-a"]))

    const doneB = queue.enqueue("codex", async () => {
      started.push("save-b")
      await saveB.promise
    })
    await Promise.resolve()
    expect(started).toEqual(["save-a", "refresh-a"])

    refreshA.resolve()
    await doneA
    await vi.waitFor(() => expect(started).toContain("save-b"))
    saveB.resolve()
    await doneB
  })
})
