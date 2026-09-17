import { readFileBase64, readWorkspaceFileBase64 } from "@/lib/api"

const RETRY_DELAYS_MS = [0, 200, 400, 800]

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function base64ToArrayBuffer(b64: string): ArrayBuffer {
  const binary = atob(b64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i)
  }
  return bytes.buffer
}

export async function readOfficeFileBytes(
  rootPath: string | null,
  relPath: string | null,
  absPath?: string | null
): Promise<ArrayBuffer> {
  if (!relPath && !absPath) {
    throw new Error("Missing file path")
  }
  const b64 =
    rootPath && relPath
      ? await readWorkspaceFileBase64(rootPath, relPath)
      : await readFileBase64(absPath ?? relPath ?? "")
  return base64ToArrayBuffer(b64)
}

export async function readOfficeFileBytesWithRetry(
  rootPath: string | null,
  relPath: string | null,
  absPath?: string | null
): Promise<ArrayBuffer> {
  let lastError: unknown
  for (const delay of RETRY_DELAYS_MS) {
    if (delay > 0) await sleep(delay)
    try {
      return await readOfficeFileBytes(rootPath, relPath, absPath)
    } catch (err) {
      lastError = err
    }
  }
  throw lastError
}
