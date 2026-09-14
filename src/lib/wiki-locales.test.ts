import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { expect, it } from "vitest"
function flatten(
  value: Record<string, unknown>,
  prefix = ""
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, item]) =>
      typeof item === "string"
        ? [[prefix + key, item]]
        : Object.entries(
            flatten(item as Record<string, unknown>, prefix + key + ".")
          )
    )
  )
}
it("has the same Wiki reading keys and interpolation variables in all ten languages", () => {
  const locales = [
    "en",
    "zh-CN",
    "zh-TW",
    "ja",
    "ko",
    "es",
    "de",
    "fr",
    "pt",
    "ar",
  ]
  const data = locales.map((locale) =>
    flatten(
      JSON.parse(
        readFileSync(resolve("src/i18n/messages", locale + ".json"), "utf8")
      ).Wiki.v2
    )
  )
  const keys = Object.keys(data[0]).sort()
  for (const [index, translations] of data.entries()) {
    expect(Object.keys(translations).sort(), locales[index]).toEqual(keys)
    for (const key of keys) {
      expect(translations[key].trim(), `${locales[index]}:${key}`).not.toBe("")
      const variables = (text: string) =>
        Array.from(text.matchAll(/\{(\w+)[},]/g), (match) => match[1]).sort()
      expect(variables(translations[key]), `${locales[index]}:${key}`).toEqual(
        variables(data[0][key])
      )
    }
  }
})
