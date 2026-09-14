import { format, type SqlLanguage } from "sql-formatter"

export type SqlFormatMode = "pretty" | "minify"

export const SQL_DIALECTS = [
  { value: "sql", label: "SQL" },
  { value: "mysql", label: "MySQL" },
  { value: "postgresql", label: "PostgreSQL" },
  { value: "sqlite", label: "SQLite" },
  { value: "mariadb", label: "MariaDB" },
  { value: "transactsql", label: "T-SQL" },
  { value: "plsql", label: "PL/SQL" },
  { value: "bigquery", label: "BigQuery" },
] as const satisfies readonly { value: SqlLanguage; label: string }[]

export type SqlFormatResult =
  | { ok: true; output: string }
  | { ok: false; error: string }

function isSqlLanguage(value: string): value is SqlLanguage {
  return SQL_DIALECTS.some((dialect) => dialect.value === value)
}

/** Collapse whitespace outside strings/comments. Never executes SQL. */
export function collapseSqlWhitespace(sql: string): string {
  let out = ""
  let i = 0
  let lastWasSpace = true
  const pushSpace = () => {
    if (!lastWasSpace) {
      out += " "
      lastWasSpace = true
    }
  }
  while (i < sql.length) {
    const c = sql[i]
    const next = sql[i + 1]
    if (c === "'" || c === '"' || c === "`") {
      const quote = c
      out += c
      lastWasSpace = false
      i += 1
      while (i < sql.length) {
        out += sql[i]
        if (sql[i] === quote) {
          if (sql[i + 1] === quote) {
            out += sql[i + 1]
            i += 2
            continue
          }
          i += 1
          break
        }
        if (quote === "'" && sql[i] === "\\" && i + 1 < sql.length) {
          out += sql[i + 1]
          i += 2
          continue
        }
        i += 1
      }
      continue
    }
    if (c === "-" && next === "-") {
      i += 2
      while (i < sql.length && sql[i] !== "\n") i += 1
      continue
    }
    if (c === "/" && next === "*") {
      i += 2
      while (i < sql.length && !(sql[i] === "*" && sql[i + 1] === "/")) i += 1
      i += 2
      continue
    }
    if (c === " " || c === "\n" || c === "\r" || c === "\t") {
      pushSpace()
      i += 1
      continue
    }
    out += c
    lastWasSpace = false
    i += 1
  }
  return out.trim()
}

export function formatSql(
  input: string,
  options: { mode: SqlFormatMode; language: string }
): SqlFormatResult {
  if (input.trim() === "") return { ok: true, output: "" }
  if (!isSqlLanguage(options.language)) {
    return { ok: false, error: `Unsupported dialect: ${options.language}` }
  }
  try {
    // Format only — sql-formatter never executes statements.
    if (options.mode === "pretty") {
      return {
        ok: true,
        output: format(input, {
          language: options.language,
          tabWidth: 2,
        }),
      }
    }
    format(input, { language: options.language })
    return { ok: true, output: collapseSqlWhitespace(input) }
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
