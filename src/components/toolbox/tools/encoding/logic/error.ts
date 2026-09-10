export class EncodingError extends Error {
  constructor(message: string) {
    super(message)
    this.name = "EncodingError"
  }
}

export function encodingErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function tryCodec(run: () => string): {
  result: string
  error: string | null
} {
  try {
    return { result: run(), error: null }
  } catch (error) {
    return { result: "", error: encodingErrorMessage(error) }
  }
}
