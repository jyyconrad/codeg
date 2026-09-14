import { dump, JSON_SCHEMA, load, YAMLException } from "js-yaml"
import { formatJsonParseError, jsonPointer } from "./json-error"

export type JsonYamlDirection = "json-to-yaml" | "yaml-to-json"

export type JsonYamlResult =
  | { ok: true; output: string }
  | { ok: false; error: string }

function yamlErrorMessage(error: unknown): string {
  if (error instanceof YAMLException) {
    const line = error.mark ? error.mark.line + 1 : undefined
    const column = error.mark ? error.mark.column + 1 : undefined
    const reason = error.reason || error.message
    if (line != null && column != null) {
      return `at line ${line}, column ${column}: ${reason}`
    }
    return reason
  }
  return error instanceof Error ? error.message : String(error)
}

function findNonJsonPath(
  value: unknown,
  segments: (string | number)[] = []
): string | null {
  if (value === null) return null
  const type = typeof value
  if (type === "string" || type === "boolean") return null
  if (type === "number") {
    return Number.isFinite(value as number) ? null : jsonPointer(segments)
  }
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      const path = findNonJsonPath(value[i], [...segments, i])
      if (path) return path
    }
    return null
  }
  if (type === "object") {
    if (Object.getPrototypeOf(value) !== Object.prototype) {
      return jsonPointer(segments)
    }
    const entries = Object.entries(value as Record<string, unknown>)
    for (const [key, child] of entries) {
      const path = findNonJsonPath(child, [...segments, key])
      if (path) return path
    }
    return null
  }
  return jsonPointer(segments)
}

function toYaml(value: unknown): JsonYamlResult {
  const bad = findNonJsonPath(value)
  if (bad) {
    return { ok: false, error: `Unsupported value at path ${bad}` }
  }
  try {
    const output = dump(value, {
      indent: 2,
      lineWidth: -1,
      noRefs: true,
      schema: JSON_SCHEMA,
      skipInvalid: false,
    })
    return { ok: true, output }
  } catch (error) {
    const path = findNonJsonPath(value)
    const message = yamlErrorMessage(error)
    return {
      ok: false,
      error: path ? `${message} (path ${path})` : message,
    }
  }
}

function fromYaml(input: string): JsonYamlResult {
  try {
    const value = load(input, { schema: JSON_SCHEMA, json: true })
    if (value === undefined) return { ok: true, output: "" }
    const bad = findNonJsonPath(value)
    if (bad) {
      return { ok: false, error: `Unsupported value at path ${bad}` }
    }
    return { ok: true, output: JSON.stringify(value, null, 2) }
  } catch (error) {
    return { ok: false, error: yamlErrorMessage(error) }
  }
}

export function convertJsonYaml(
  input: string,
  direction: JsonYamlDirection
): JsonYamlResult {
  if (input.trim() === "") return { ok: true, output: "" }
  if (direction === "json-to-yaml") {
    try {
      const value = JSON.parse(input) as unknown
      return toYaml(value)
    } catch (error) {
      return { ok: false, error: formatJsonParseError(error, input) }
    }
  }
  return fromYaml(input)
}
